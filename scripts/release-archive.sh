#!/bin/sh
# Builds the release archive of one architecture: dist/lmx-<version>-<arch>-linux.tar.gz and its
# SHA-256 file. `task release/build` runs it from the repository root in the ci/rust container.
#
# Usage: release-archive.sh VERSION ARCH
set -eu

version=$1
arch=$2

target="$arch-unknown-linux-musl"
name="lmx-$version-$arch-linux"
stage="dist/$name"

cargo build --release --locked --target "$target" -p lmx -p lmxd

rm -rf "$stage" "$stage.tar" "$stage.tar.gz" "$stage.tar.gz.sha256"
mkdir -p "$stage"
install -m 0755 "$CARGO_TARGET_DIR/$target/release/lmx" "$stage/lmx"
install -m 0755 "$CARGO_TARGET_DIR/$target/release/lmxd" "$stage/lmxd"
install -m 0644 LICENSE "$stage/LICENSE"

tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -C dist -cf "$stage.tar" "$name"
gzip -9n "$stage.tar"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
rm -rf "$stage"
