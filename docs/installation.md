# Installation

Prebuilt binaries are published on [GitHub Releases](https://github.com/yidika-lab/yidika/releases) for every tagged version. The install script picks the right one for your machine, checks its SHA-256 checksum and puts `yidi` in `~/.yidi/bin`. Nothing outside your home folder changes and no administrator rights are needed.

## Linux and macOS

```sh
curl -fsSL https://raw.githubusercontent.com/yidika-lab/yidika/main/install.sh | sh
```

## Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/yidika-lab/yidika/main/install.ps1 | iex
```

The script then:

1. checks the archive against its `.sha256` file;
2. runs a test program and prints `self-test passed: Hello from Yidika!`;
3. adds `~/.yidi/bin` to your `PATH` for future terminals (`~/.zshrc`, `~/.bashrc`, fish's `conf.d` or `~/.profile` depending on your shell; the user `PATH` on Windows);
4. makes `yidi` usable right away. On Linux and macOS a script piped into `sh` cannot change the shell that started it, so the installer opens a new shell where `yidi` works; type `exit` to go back.

## Supported systems

| OS | Architectures | Requirements |
|---|---|---|
| Linux | x86_64, aarch64 | glibc 2.35 or later (Ubuntu 22.04, Debian 12, Fedora 36 and newer) |
| macOS | Apple Silicon, Intel | macOS 11 or later |
| Windows | x86_64 (ARM64 through Windows 11 emulation) | Windows 10 or 11, PowerShell 5.1 or later |

The interpreter works everywhere in this table. Native compilation, which `yidi run` uses by default, also needs a C++ compiler: `clang` on Linux (`sudo apt install clang`), the Xcode Command Line Tools on macOS (`xcode-select --install`), LLVM and the Visual Studio Build Tools on Windows. Without one, `yidi run` uses the interpreter. See [Interpreter and native code](engines.md).

## Options

| Task | Linux and macOS | Windows |
|---|---|---|
| A specific version | `curl -fsSL .../install.sh \| YIDI_VERSION=v0.2.0 sh` | `$env:YIDI_VERSION='v0.2.0'; irm .../install.ps1 \| iex` |
| Leave the shell profile alone | `YIDI_NO_MODIFY_PATH=1` before `sh` | not needed |
| No new shell at the end | `YIDI_NO_SHELL=1` before `sh` | not applicable |
| Another folder | `YIDI_HOME=/opt/yidi` before `sh` | `$env:YIDI_HOME='D:\tools\yidi'` |

## Update and uninstall

```sh
yidi upgrade --check   # is there a newer release?
yidi upgrade           # download it, check its SHA-256, replace yidi
```

To uninstall, delete `~/.yidi` (`Remove-Item -Recurse $HOME\.yidi` on Windows), then remove the `# yidi` line and the line after it from your shell profile, or the entry from your user `PATH` on Windows.

## Without the script

Download the archive for your platform from the [latest release](https://github.com/yidika-lab/yidika/releases/latest), check it with `sha256sum -c`, `shasum -a 256 -c` or `Get-FileHash`, extract it and put `yidi` (on Windows `yidi.exe` and `yk_ffi_gui.dll`) in a folder on your `PATH`.

## From source

With [Rust](https://rustup.rs) stable and a C toolchain:

```sh
cargo install --git https://github.com/yidika-lab/yidika yidi
```
