{
  description = "loker: self-hosted job aggregator with eligibility, MCP, REST and CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      build = pkgs:
        let
          craneLib = crane.mkLib pkgs;
          # Cargo sources, the SQL migrations embedded by `sqlx::migrate!`, and the
          # committed contract schemas the e2e test compares against.
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              (pkgs.lib.hasSuffix ".sql" path)
              || (pkgs.lib.hasInfix "/schema" path)
              || (craneLib.filterCargoSources path type);
          };
          commonArgs = {
            inherit src;
            pname = "loker";
            strictDeps = true;
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          package = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            meta.mainProgram = "loker";
          });
          clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });
          fmt = craneLib.cargoFmt { inherit src; pname = "loker"; };
        };
    in
    {
      packages = forAll (pkgs:
        let b = build pkgs; in {
          default = b.package;
        });

      checks = forAll (pkgs:
        let b = build pkgs; in {
          inherit (b) package clippy fmt;
          comments = pkgs.runCommand "loker-comment-ratchet" { } ''
            ${pkgs.python3}/bin/python3 ${self}/lint/comment_ratchet.py --root ${self} --check
            touch $out
          '';
        });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer sqlite litestream secretspec python3 ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
    };
}
