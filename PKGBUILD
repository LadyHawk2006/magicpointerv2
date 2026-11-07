# Maintainer: Lex

pkgname=magicpointerv2
pkgver=1.0.0
pkgrel=1
pkgdesc='Ask Gemini questions about selected screen regions on Hyprland'
arch=('x86_64')
url='https://github.com/LadyHawk2006/magicpointerv2'
license=('MIT')

# This package intentionally installs the upstream prebuilt release binary;
# it does not compile Rust sources or require a Rust toolchain.
depends=(
  'gtk4'
  'gtk4-layer-shell'
  'hyprland'
  'libsecret'
  'mesa'
  'wayland'
)
optdepends=(
  'gnome-keyring: Secret Service provider for storing Gemini API keys'
  'keepassxc: alternative Secret Service provider'
)

_release_tag='v.1.0.0'
_release_url="https://github.com/LadyHawk2006/magicpointerv2/releases/download/${_release_tag}/magicpointerv2"
_raw_url="https://raw.githubusercontent.com/LadyHawk2006/magicpointerv2/${_release_tag}"

source=(
  "${pkgname}::${_release_url}"
  "magicpointerv2.desktop::${_raw_url}/Assets/com.magicpointerv2.tool.desktop"
  "magicpointerv2.svg::${_raw_url}/Assets/icon.svg"
  "magicpointerv2.metainfo.xml::${_raw_url}/Assets/com.magicpointerv2.tool.metainfo.xml"
  "magicpointerv2-README.md::${_raw_url}/README.md"
  "magicpointerv2-LICENSE::${_raw_url}/LICENSE"
)
sha256sums=(
  '69377481a3b0e63d6a3911fb891584a9cfb7054f3789cdbe1b3e199e20bbcbf3'
  '6eabad3b80d79f42620623ba8229861fcc01462d87596a17cb503b518a3d8c5e'
  '8339725c00e9017dcdee2816f5573daa06b529749e6f01d0e57092c062005f90'
  'e3356e5a9a191ecf9f5178f5acd81806df5557ef42b960660d90ffb6f591899e'
  '540c255d61ea31bb0d81655fb6f616d1b19ef0730555ac0b1c5553096e13d2b2'
  'fd35a6c038d31ba19d5d31188392e5a633ea6324cce66d55e06d835518a3ef8f'
)

package() {
  # Install the prebuilt x86_64 executable.
  install -Dm755 "${srcdir}/${pkgname}" \
    "${pkgdir}/usr/bin/${pkgname}"

  # Register the app with desktop menus and themed icon lookup.
  install -Dm644 "${srcdir}/magicpointerv2.desktop" \
    "${pkgdir}/usr/share/applications/com.magicpointerv2.tool.desktop"
  install -Dm644 "${srcdir}/magicpointerv2.svg" \
    "${pkgdir}/usr/share/icons/hicolor/scalable/apps/com.magicpointerv2.tool.svg"

  # Install AppStream metadata for software centers and package discovery.
  install -Dm644 "${srcdir}/magicpointerv2.metainfo.xml" \
    "${pkgdir}/usr/share/metainfo/com.magicpointerv2.tool.metainfo.xml"

  # Include usage and license information for package users.
  install -Dm644 "${srcdir}/magicpointerv2-README.md" \
    "${pkgdir}/usr/share/doc/${pkgname}/README.md"
  install -Dm644 "${srcdir}/magicpointerv2-LICENSE" \
    "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"
}
