#!/bin/sh
# Yidika installer for Linux and macOS.
#   curl -fsSL https://raw.githubusercontent.com/yidika-lab/yidika/main/install.sh | sh
# Environment:
#   YIDI_VERSION  release tag to install (default: latest), e.g. v0.1.0
#   YIDI_HOME     install prefix (default: $HOME/.yidi); the binary goes to $YIDI_HOME/bin
#   YIDI_NO_MODIFY_PATH=1  do not add the bin directory to your shell profile
set -eu

REPO="yidika-lab/yidika"

say() { printf 'yidi-install: %s\n' "$1"; }
fail() { printf 'yidi-install: error: %s\n' "$1" >&2; exit 1; }

need() {
    command -v "$1" > /dev/null 2>&1 || fail "'$1' is required but was not found"
}

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$arch" in
        x86_64 | amd64) arch="x86_64" ;;
        aarch64 | arm64) arch="aarch64" ;;
        *) fail "unsupported CPU architecture: $arch" ;;
    esac
    case "$os" in
        Linux) echo "${arch}-unknown-linux-gnu" ;;
        Darwin) echo "${arch}-apple-darwin" ;;
        *) fail "unsupported OS: $os (on Windows use install.ps1)" ;;
    esac
}

download() {
    if command -v curl > /dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
    elif command -v wget > /dev/null 2>&1; then
        wget --https-only -q "$1" -O "$2"
    else
        fail "curl or wget is required"
    fi
}

sha256_of() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum > /dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "sha256sum or shasum is required to verify the download"
    fi
}

release_url() {
    version="${YIDI_VERSION:-latest}"
    if [ "$version" = "latest" ]; then
        echo "https://github.com/${REPO}/releases/latest/download"
    else
        echo "https://github.com/${REPO}/releases/download/${version}"
    fi
}

bin_dir_for_profile() {
    if [ "$1" = "$HOME/.yidi" ]; then
        echo '$HOME/.yidi/bin'
    else
        echo "$1/bin"
    fi
}

shell_profiles() {
    case "$(basename "${SHELL:-sh}")" in
        zsh) echo "${ZDOTDIR:-$HOME}/.zshrc" ;;
        bash)
            echo "$HOME/.bashrc"
            if [ "$(uname -s)" = "Darwin" ]; then
                echo "$HOME/.bash_profile"
            fi
            ;;
        fish) echo "$HOME/.config/fish/conf.d/yidi.fish" ;;
        *) echo "$HOME/.profile" ;;
    esac
}

path_line() {
    case "$1" in
        *.fish) echo "fish_add_path -g \"$2\"" ;;
        *) echo "export PATH=\"$2:\$PATH\"" ;;
    esac
}

add_line_to_profile() {
    if grep -qsF "$2" "$1"; then
        return
    fi
    mkdir -p "$(dirname "$1")"
    printf '\n# yidi\n%s\n' "$2" >> "$1"
    say "added yidi to your PATH in $1"
}

configure_path() {
    if [ "${YIDI_NO_MODIFY_PATH:-0}" = "1" ]; then
        return
    fi
    dir="$(bin_dir_for_profile "$1")"
    shell_profiles | while IFS= read -r profile; do
        add_line_to_profile "$profile" "$(path_line "$profile" "$dir")"
    done
}

path_hint() {
    if [ "${YIDI_NO_MODIFY_PATH:-0}" = "1" ]; then
        echo "add yidi to your PATH:"
    else
        echo "open a new terminal to use yidi, or run now:"
    fi
}

main() {
    need uname
    need tar
    target="$(detect_target)"
    asset="yidi-${target}.tar.gz"
    base="$(release_url)"
    prefix="${YIDI_HOME:-$HOME/.yidi}"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT TERM

    say "downloading ${asset} (${YIDI_VERSION:-latest})"
    download "${base}/${asset}" "${tmp}/${asset}"
    download "${base}/${asset}.sha256" "${tmp}/${asset}.sha256"

    expected="$(cut -d ' ' -f 1 < "${tmp}/${asset}.sha256")"
    actual="$(sha256_of "${tmp}/${asset}")"
    [ "$expected" = "$actual" ] || fail "checksum mismatch for ${asset}"

    tar -xzf "${tmp}/${asset}" -C "$tmp"
    mkdir -p "${prefix}/bin"
    install -m 755 "${tmp}/yidi-${target}/yidi" "${prefix}/bin/yidi"

    say "installed $("${prefix}/bin/yidi" --version) to ${prefix}/bin/yidi"
    configure_path "$prefix"
    case ":${PATH}:" in
        *":${prefix}/bin:"*) ;;
        *) say "$(path_hint)  export PATH=\"${prefix}/bin:\$PATH\"" ;;
    esac
}

main "$@"
