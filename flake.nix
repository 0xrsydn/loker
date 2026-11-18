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
          # Cargo sources plus the SQL migrations embedded by `sqlx::migrate!`.
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type: (pkgs.lib.hasSuffix ".sql" path) || (craneLib.filterCargoSources path type);
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
        };
    in
    {
      packages = forAll (pkgs:
        let b = build pkgs; in {
          default = b.package;
        });

      # `nix flake check`: the build plus clippy with warnings denied.
      checks = forAll (pkgs:
        let b = build pkgs; in {
          inherit (b) package clippy;
        });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer sqlite litestream secretspec ];
          # rust-analyzer needs the std sources of this exact toolchain.
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
    };
}
