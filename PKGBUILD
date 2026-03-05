# Maintainer: Your Name <your.email@example.com>
pkgname=remote-desktop-wayland
pkgver=0.1.0
pkgrel=1
pkgdesc="Custom remote desktop for KDE Plasma Wayland with Tailscale-only access"
arch=(x86_64)
url="https://github.com/your/remote-desktop-wayland"
license=('MIT' OR 'Apache-2.0')
depends=(
    # Runtime dependencies
    'pipewire' 'wireplumber'
    'xdg-desktop-portal-kde'
    'tailscale'
    'nftables'
    'libei'
    'ffmpeg'
    'gcc-libs' 'glibc'
)
makedepends=('cargo' 'rust')
optdepends=(
    'krfb: Alternative VNC backend'
)
backup=(
    'etc/remote-desktop/config.toml'
)

prepare() {
    cd "$srcdir"
    if [ -d "$pkgname" ]; then
        rm -rf "$pkgname"
    fi
    cp -r "$startdir/$pkgname" "$pkgname"
    cd "$pkgname"

    export RUSTUP_TOOLCHAIN=stable
    cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
    cd "$srcdir/$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    export CARGO_TARGET_DIR=target
    cargo build --frozen --release --all-features
}

check() {
    cd "$srcdir/$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    cargo test --frozen --all-features
}

package() {
    cd "$srcdir/$pkgname"

    # Install binaries
    install -Dm755 "target/release/remote-desktop-server" "$pkgdir/usr/bin/remote-desktop-server"
    install -Dm755 "target/release/remote-desktop" "$pkgdir/usr/bin/remote-desktop"

    # Install systemd units
    install -Dm644 "systemd/remote-desktop.service" \
        "$pkgdir/usr/lib/systemd/user/remote-desktop.service"
    install -Dm644 "systemd/remote-desktop.socket" \
        "$pkgdir/usr/lib/systemd/user/remote-desktop.socket"

    # Install nftables rules
    install -Dm644 "nftables-rules.conf" \
        "$pkgdir/etc/nftables.d/remote-desktop.conf"

    # Install example config
    install -Dm644 "config/remote-desktop.conf.example" \
        "$pkgdir/etc/remote-desktop/config.toml.example"

    # Install documentation
    install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}

# vim:set ts=2 sw=2 et:
