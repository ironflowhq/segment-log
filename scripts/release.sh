#!/bin/bash
# Publishes a release: both crates to crates.io, then the `vVERSION` and `clients/go/vVERSION` tags.
# The `vVERSION` tag starts .github/workflows/release.yml, which builds the musl server binary and
# creates the GitHub release. A crates.io version can never be republished, so the script refuses
# to start unless the tree is exactly origin/main with green CI, and it skips every step that is
# already done, so a rerun after a failure is safe.
# Usage: scripts/release.sh VERSION [--dry-run]    (needs `cargo login` and `gh auth login`)
set -euo pipefail
version="${1:?usage: release.sh VERSION [--dry-run]}"
dry_run="${2:-}"
[ -z "$dry_run" ] || [ "$dry_run" = --dry-run ] || { echo "unknown option: $dry_run" >&2; exit 2; }
cd "$(git rev-parse --show-toplevel)"

fail() { echo "release: $*" >&2; exit 1; }
run() { if [ -n "$dry_run" ]; then echo "would run: $*"; else echo "+ $*"; "$@"; fi; }
published() {
	curl -sf -o /dev/null -A "segment-log-release" "https://crates.io/api/v1/crates/$1/$version"
}

git fetch -q --tags origin
[ "$(git branch --show-current)" = main ] || fail "not on main"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "uncommitted changes"
head="$(git rev-parse HEAD)"
[ "$head" = "$(git rev-parse origin/main)" ] || fail "HEAD is not origin/main; pull or push first"
cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
[ "$cargo_version" = "$version" ] || fail "Cargo.toml says $cargo_version, not $version"
grep -q "^## $version " CHANGELOG.md || fail "CHANGELOG.md has no '## $version ' section"
ci="$(gh run list --workflow CI --commit "$head" --json conclusion -q '.[0].conclusion')"
[ "$ci" = success ] || fail "CI on ${head:0:7} is '${ci:-not run}', not success"
for tag in "v$version" "clients/go/v$version"; do
	if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
		[ "$(git rev-list -n 1 "$tag")" = "$head" ] || fail "tag $tag exists on another commit"
	fi
done

# The server depends on the core at this version, so the core goes first. `cargo publish` waits
# until the index serves the new version before it returns.
for crate in segment-log-core segment-log-server; do
	if published "$crate"; then
		echo "$crate $version is already on crates.io"
	elif [ -n "$dry_run" ] && [ "$crate" = segment-log-server ] && ! published segment-log-core; then
		echo "would run: cargo publish -p $crate (its dry run needs segment-log-core $version on crates.io)"
	elif [ -n "$dry_run" ]; then
		cargo publish -p "$crate" --dry-run
	else
		run cargo publish -p "$crate"
	fi
done

for tag in "v$version" "clients/go/v$version"; do
	git rev-parse -q --verify "refs/tags/$tag" >/dev/null || run git tag "$tag" "$head"
done
run git push origin "v$version" "clients/go/v$version"
[ -n "$dry_run" ] || echo "release.yml is building the GitHub release: gh run list --workflow release -L 1"
