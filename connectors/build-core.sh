# The part of every connector's build.sh that is the same: get the core's source at its
# pinned commit, apply the connector, write the build id, build, and copy the library out.
# Not run by itself: a connector's build.sh sets what is its own and then sources this file.
#
#   REPO        the core's git repository
#   COMMIT      the commit to build, in full
#   CLEAN       the folders of the source tree the connector adds files to or builds in,
#               emptied of untracked files before each build
#   CLEAN_IGNORED
#               optional, 1: also empty them of files the core's .gitignore names (a core
#               whose makefile misses changed headers)
#   ID_HEADER   the header to write the build id into, as the connector includes it
#   BUILT       where the build leaves the library, in the source tree, without its suffix
#   compile     a function that builds the core. It runs at the top of the source tree, and
#               $jobs is how many jobs to run at once
#
# The connector's name is its folder's: connectors/<name>/ holds dj-<name>.patch and src/,
# the source goes to build/<name>-src/, and the build id starts `<name>@`.
#
# Usage, through a build.sh:
#   connectors/<name>/build.sh [output-dir]   build the core into output-dir (build/cores/)
#   connectors/<name>/build.sh --id           print the build id and do nothing else
#   connectors/<name>/build.sh --source DIR   write the source the core is built from into
#                                             DIR as <name>-<commit>-dj.<id>-source.tar.gz,
#                                             without building: the core's tree at the
#                                             commit with the connector applied, and the
#                                             connector's folder in dj-connector/

here="$(cd "$(dirname "$0")" && pwd)"
name="$(basename "$here")"
patch="$here/dj-$name.patch"
work="$here/../../build/$name-src"
mode=build
case "${1:-}" in
  --id) mode=id ;;
  --source) mode=source; out="${2:?--source needs a folder to write into}" ;;
  *) out="${1:-$here/../../build/cores}" ;;
esac

# decomp-dj's own files (MIT, src/LICENSE.txt), laid out as in the core's tree. The patch
# only hooks them into the core's files.
own="$(cd "$here/src" && find . -type f ! -name LICENSE.txt | LC_ALL=C sort)"
# The build id the connector reports (dj_build_id): the upstream commit, then the patch and
# the files under src/ in that order, line endings aside, so the same source gives the same
# id on every platform. Nothing else goes into it: not this file, nor build.sh.
patch_id="$({ cat "$patch"; for f in $own; do cat "$here/src/$f"; done; } | tr -d '\r' | { shasum -a 256 2>/dev/null || sha256sum; } | cut -c1-8)"
build_id="$name@${COMMIT:0:8}+dj.$patch_id"
if [ "$mode" = id ]; then
  echo "$build_id"
  exit 0
fi

mkdir -p "$out"
out="$(cd "$out" && pwd)"
if [ ! -d "$work/.git" ]; then
  git clone "$REPO" "$work"
fi
cd "$work"
git fetch --depth 1 origin "$COMMIT" 2>/dev/null || true
git checkout -q --force "$COMMIT"
# shellcheck disable=SC2086
git clean -qfd${CLEAN_IGNORED:+x} $CLEAN
git apply "$patch"
for f in $own; do
  mkdir -p "$(dirname "$f")"
  cp "$here/src/$f" "$f"
done
printf '#define DJ_BUILD_ID "%s"\n' "$build_id" > "$ID_HEADER"

if [ "$mode" = source ]; then
  # Every file git tracks, as the patch left it, with the connector's own files and the
  # build id, and the connector's folder beside them. Made through a tree in a scratch
  # index, so build leftovers stay out and the checkout's own index is untouched.
  base="$name-${COMMIT:0:8}-dj.$patch_id-source"
  rm -rf dj-connector
  mkdir -p "dj-connector/$name"
  cp -R "$here/." "dj-connector/$name/"
  cp "$here/../build-core.sh" dj-connector/
  cat > dj-connector/README.txt <<NOTE
The source of the libretro core decomp-dj ships as $build_id:
$REPO at commit $COMMIT, with decomp-dj's connector applied
(dj-connector/$name/dj-$name.patch and the files under dj-connector/$name/src/).

To build it, run this tree's build as upstream describes, or from a decomp-dj checkout
run connectors/$name/build.sh, which fetches the same commit and applies the same files.
The licences: dj-connector/$name/LICENSE.txt (the core, and the patch) and
dj-connector/$name/src/LICENSE.txt (the connector's own files).
NOTE
  index="$(mktemp)"
  tree="$(
    export GIT_INDEX_FILE="$index"
    git read-tree HEAD
    git add -u
    # shellcheck disable=SC2086
    git add -f -- $own "$ID_HEADER" dj-connector
    git write-tree
  )"
  git archive --format=tar --prefix="$base/" "$tree" | gzip -n > "$out/$base.tar.gz"
  rm -rf "$index" dj-connector
  echo "wrote $out/$base.tar.gz"
  exit 0
fi

jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"
compile

made=""
for f in "$BUILT.so" "$BUILT.dylib" "$BUILT.dll"; do
  if [ -f "$f" ]; then
    # Copy to a new file and move it into place. Overwriting the old file in place keeps
    # its inode, and macOS then kills any process that loads it (Code Signature Invalid),
    # because it cached the old file's signature.
    cp "$f" "$out/.$(basename "$f").new"
    mv -f "$out/.$(basename "$f").new" "$out/$(basename "$f")"
    echo "built $out/$(basename "$f") ($build_id)"
    made=1
  fi
done
if [ -z "$made" ]; then
  echo "$name: the build finished but left no $BUILT.so, .dylib or .dll" >&2
  exit 1
fi
