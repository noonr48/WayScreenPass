# Maintainer: Your Name <your.email@example.com>
pkgname=wayscreenpass
pkgver=0.1.0
pkgrel=1
pkgdesc="Headless Wayland remote desktop over Tailscale"
arch=(x86_64)
url="https://github.com/noonr48/WayScreenPass"
license=('MIT' 'Apache-2.0')
depends=(
    'tailscale'
    'nftables'
    'sway'
    'grim'
    'wl-clipboard'
    'x264'
    'ffmpeg'
    'sdl2'
    'gcc-libs'
    'glibc'
)
makedepends=('cargo' 'rust')
source=("$pkgname::git+$url.git")
sha256sums=('SKIP')

build() {
    cd "$srcdir/$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    cargo build --frozen --release --workspace
}

check() {
    cd "$srcdir/$pkgname"
    export RUSTUP_TOOLCHAIN=stable
    cargo test --frozen --workspace
}

package() {
    cd "$srcdir/$pkgname"

    install -Dm755 "target/release/remote-desktop-server" "$pkgdir/usr/bin/remote-desktop-server"
    install -Dm755 "target/release/remote-desktop" "$pkgdir/usr/bin/remote-desktop"
    install -Dm755 "target/release/remote-desktop-tray" "$pkgdir/usr/bin/remote-desktop-tray"

    install -Dm644 "systemd/remote-desktop.service" \
        "$pkgdir/usr/lib/systemd/user/remote-desktop.service"

    install -Dm644 "nftables-rules.conf" \
        "$pkgdir/usr/share/$pkgname/nftables-rules.conf"

    install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
}

# vim:set ts=2 sw=2 et:
