#!/usr/bin/env bash
set -euo pipefail

bucket=loker-nix-cache
for v in AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY R2_ENDPOINT NIX_SIGNING_KEY; do
  if [ -z "${!v:-}" ]; then
    echo "push-nix-cache: $v is not set, skipping the cache push"
    exit 0
  fi
done

system=$(nix eval --raw --impure --expr builtins.currentSystem)
mapfile -t drvs < <(nix path-info --derivation ".#checks.$system.package" ".#checks.$system.clippy")

mapfile -t candidates < <(
  nix-store --query --requisites --include-outputs "${drvs[@]}" |
    grep -v '\.drv$' |
    grep -vE -- '-(loker|loker-clippy|loker-fmt)-[0-9][^/]*$|-source$' |
    while read -r p; do [ -e "$p" ] && echo "$p"; done
)

mapfile -t push < <(
  printf '%s\n' "${candidates[@]}" |
    xargs -P 32 -n 1 sh -c 'h=$(basename "$1" | cut -c1-32)
      curl -sfI -o /dev/null "https://cache.nixos.org/$h.narinfo" || echo "$1"' _
)
echo "push-nix-cache: ${#push[@]} of ${#candidates[@]} paths are not on cache.nixos.org"
[ "${#push[@]}" -gt 0 ] || exit 0

key=$(mktemp)
trap 'rm -f "$key"' EXIT
printf '%s' "$NIX_SIGNING_KEY" > "$key"
to=${NIX_CACHE_TO:-"s3://$bucket?endpoint=$R2_ENDPOINT&region=auto"}
case $to in *\?*) to="$to&" ;; *) to="$to?" ;; esac
# Copies each path's runtime references too (glibc, ...): a binary cache only
# accepts a path whose references it holds. Those change only with nixpkgs.
nix copy --to "${to}compression=zstd&secret-key=$key" "${push[@]}"
