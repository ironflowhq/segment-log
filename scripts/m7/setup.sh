# Sourced by scripts/m7-rehearse.sh and scripts/m7-power-cut.sh (ADR 0059). Fetches a pinned
# Firecracker and CI guest kernel and rootfs, builds the musl server and the powercut tool, and
# puts the server in the rootfs with scripts/m7/guest-init. Sets repo and M7_CACHE.

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
export M7_CACHE=${M7_CACHE:-$HOME/.cache/segment-log-m7}
fc_version=${M7_FC_VERSION:-v1.18.0}
ci=${M7_CI:-v1.15} # newest Firecracker CI prefix with guest artifacts on 2026-11-15
kernel=${M7_KERNEL:-6.1.155}
arch=$(uname -m)
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/.cache/segment-log-target} # the repo may be a read-only mount
export PATH=$HOME/.cargo/bin:/usr/local/go/bin:$PATH
[ -w /dev/kvm ] || { echo "no writable /dev/kvm" >&2; exit 1; }
mkdir -p "$M7_CACHE"
cd "$M7_CACHE"

if [ ! -x firecracker ] || ! ./firecracker --version | grep -q "Firecracker $fc_version\$"; then
	curl -fsSL "https://github.com/firecracker-microvm/firecracker/releases/download/$fc_version/firecracker-$fc_version-$arch.tgz" | tar -xz
	cp "release-$fc_version-$arch/firecracker-$fc_version-$arch" firecracker
fi
# fetch <local name> <name under the CI prefix>: the CI prefix is not versioned per file, so each
# artifact is pinned by hash in guest-artifacts.sha256 and a changed or missing one stops setup.
fetch() {
	local want
	want=$(awk -v k="$ci/$arch/$2" '$2 == k { print $1 }' "$repo/scripts/m7/guest-artifacts.sha256")
	[ -n "$want" ] || { echo "no pinned hash for $ci/$arch/$2 in scripts/m7/guest-artifacts.sha256" >&2; exit 1; }
	[ -s "$M7_CACHE/$1" ] ||
		curl -fsSL -o "$M7_CACHE/$1" "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/$ci/$arch/$2"
	[ "$(sha256sum "$M7_CACHE/$1" | cut -d' ' -f1)" = "$want" ] ||
		{ echo "$1 does not match its pinned hash for $ci/$arch/$2" >&2; exit 1; }
}
fetch "vmlinux-$kernel" "vmlinux-$kernel"
ln -sf "vmlinux-$kernel" vmlinux
fetch "ubuntu-$ci.squashfs" ubuntu-24.04.squashfs

echo "== build the server"
(cd "$repo" && rustup target add "$arch-unknown-linux-musl" >/dev/null &&
	cargo build -q --release -p segment-log-server --target "$arch-unknown-linux-musl" &&
	cargo build -q --release -p segment-log-server --target "$arch-unknown-linux-musl" \
		--features rewrite-control --target-dir "$CARGO_TARGET_DIR/rewrite-control")
server=$CARGO_TARGET_DIR/$arch-unknown-linux-musl/release/segment-log-server
control_server=$CARGO_TARGET_DIR/rewrite-control/$arch-unknown-linux-musl/release/segment-log-server

echo "== build the powercut tool"
# Static, because the guest runs it too, as the flush probe.
(cd "$repo/clients/go" && CGO_ENABLED=0 go build -o "$M7_CACHE/powercut" ./cmd/powercut)

echo "== build the guest rootfs"
sudo rm -rf rootfs.d
sudo unsquashfs -q -d rootfs.d "ubuntu-$ci.squashfs" >/dev/null
sudo mkdir -p rootfs.d/mnt && sudo install -m 0755 "$server" rootfs.d/segment-log-server
sudo install -m 0755 "$control_server" rootfs.d/segment-log-server-control
sudo install -m 0755 "$repo/scripts/m7/guest-init" rootfs.d/segment-log-init
sudo install -m 0755 "$M7_CACHE/powercut" rootfs.d/segment-log-probe
sudo install -m 0644 "$repo/scripts/m7/fingerprint" rootfs.d/etc/m7-fingerprint
rm -f rootfs.ext4
truncate -s 1G rootfs.ext4
sudo /sbin/mkfs.ext4 -q -F -d rootfs.d rootfs.ext4
sudo chown "$(id -u):$(id -g)" rootfs.ext4

cd "$repo"
