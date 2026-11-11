# Maintainer: Antonio Rojas <arojas@archlinux.org>
# Maintainer: Felix Yan <felixonmars@archlinux.org>
# Contributor: Andrea Scarpino <andrea@archlinux.org>

pkgname=qt6-declarative
_pkgver=6.12.0
pkgver=${_pkgver/-/}
pkgrel=4
arch=(x86_64)
url='https://www.qt.io'
license=(GPL-3.0-only
         LGPL-3.0-only
         LicenseRef-Qt-Commercial
         Qt-GPL-exception-1.0)
pkgdesc='Classes for QML and JavaScript languages'
depends=(glibc
         libgcc
         libstdc++
         qt6-base)
makedepends=(cmake
             git
             ninja
             python
             qt6-languageserver
             qt6-shadertools
             qt6-svg
             vulkan-headers)
optdepends=('qt6-svg: for QtQuickVectorImage and svgtoqml')
groups=(qt6)
_pkgfn=${pkgname/6-/}
source=(git+https://code.qt.io/qt/$_pkgfn#tag=v$_pkgver)
sha256sums=('2abdb7dc4300da123b730ff70e11f5c3b8643dc27ce9efcb906bd8670fbf17e7')

prepare() {
  git -C $_pkgfn cherry-pick -n 8c23f65ec4889f9ff7f609002a129698796de2b8 # Fix QML errors in Plasma
  git -C $_pkgfn cherry-pick -n 7214a32024be080099ad05c596ee778623ed66d8 # Fix hang on Plasma Logout
  git -C $_pkgfn cherry-pick -n 88a046d8875115ea9d7cf4e825add6809e8f3739 # Fix enum key shadowing
}

build() {
  cmake -B build -S $_pkgfn -G Ninja \
    -DCMAKE_MESSAGE_LOG_LEVEL=STATUS \
    -DCMAKE_CXX_FLAGS="$CXXFLAGS -ffat-lto-objects"
  cmake --build build
}

package() {
  DESTDIR="$pkgdir" cmake --install build

  install -d "$pkgdir"/usr/share/licenses
  ln -s /usr/share/licenses/qt6-base "$pkgdir"/usr/share/licenses/$pkgname
}
