#!/bin/sh
# Publish one plugin: build its package and attach it to a GitHub release tagged <id>-v<version>.
#   scripts/release.sh track
# The release notes are the plugin's description plus how to install it. Needs gh (logged in).
set -eu
cd "$(dirname "$0")/.."
id=${1:?usage: scripts/release.sh <plugin-id>}
version=$(node -p "require('./$id/plugin.json').version")
name=$(node -p "require('./$id/plugin.json').name")
desc=$(node -p "require('./$id/plugin.json').description")
plannr=$(node -p "require('./$id/plugin.json').plannr || ''")
tag="$id-v$version"
git diff --quiet && git diff --cached --quiet || { echo "commit your changes first" >&2; exit 1; }
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { echo "$tag already exists: raise the version in $id/plugin.json" >&2; exit 1; }
scripts/package.sh "$id"
pkg="dist/plannr-plugin-$id-$version.plannr-plugin"
notes=$(mktemp)
cat > "$notes" <<NOTES
$desc

**Install:** download \`plannr-plugin-$id-$version.plannr-plugin\` below, then in Plannr open **Plugins → Manage plugins… → Install from file** and pick it (or drop it on that dialog). Switch it off or remove it in the same dialog.

${plannr:+Needs Plannr $plannr or newer. }Documentation: [$id/README.md](https://github.com/MatthiasDM/plannr-plugins/blob/$tag/$id/README.md)
NOTES
git tag -a "$tag" -m "$name $version"
git push origin "$tag"
gh release create "$tag" "$pkg" --title "$name $version" --notes-file "$notes"
rm "$notes"
