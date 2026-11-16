# Packages and the registry

## A project

```sh
yidi init my-app
```

creates:

```text
my-app/
  project.toml
  main.yk
  .gitignore      (contains @yidi/)
```

`project.toml` names the project and lists its dependencies:

```toml
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
greet = "^1.2"
```

## Adding and removing

```sh
yidi add greet               # newest release in the registry, saved as greet = "^x.y.z"
yidi add greet@^1.2          # a range
yidi add github:user/greet   # any git repository, without the registry
yidi remove greet
```

A dependency can be written three ways in `project.toml`:

```toml
[dependencies]
greet = "^1.2"                                         # from the registry
local = "https://github.com/user/local"                # a repository, newest tag
pinned = { git = "https://github.com/user/pinned", version = "~0.3" }
```

Ranges follow semver: `^1.2` allows `1.x` from `1.2.0`, `~0.3` allows `0.3.x`, `=1.0.4` one release. The dependencies of a package are installed too, and the highest version that satisfies every range wins.

## Where packages go

Installed packages are in `@yidi/<name>/` at the project root, and imported by name:

```yidika-file
use { hello } from "greet";
```

Downloads go to one cache shared by all projects (`~/.cache/yidi/packages` on Linux), and `@yidi/` is made of hard links into it. A second project, or a fresh checkout, installs without the network. Measured on Linux with a warm cache: reinstalling from scratch took 1 ms, and `yidi sync` with nothing to do took 0 ms.

## The lock file

`yidi.lock` records, for every package, its source, version, commit and the SHA-256 of its files. Commit it. `yidi sync` installs exactly that, and refuses a download whose hash differs. `yidi update` is the only command that moves versions.

## Publishing

A package is a git repository with a `project.toml` that has a `[package]` section, and a `<name>.yk` or `main.yk` entry file. Releases are tags such as `v1.2.0`. Only functions marked `export` are visible to users of the package.

```sh
yidi publish --dry-run       # check everything and print the registry entry
yidi publish                 # tag v<version>, push it, open the registry pull request
```

`yidi publish` needs a clean working tree, an `https` (or GitHub SSH) `origin`, and an entry file that type-checks. With the GitHub CLI logged in it opens the pull request itself; otherwise it prints a link that fills it in.

## The registry

The registry is the git repository [`yidika-lab/registry`](https://github.com/yidika-lab/registry), browsable at [yidika-lab.github.io/registry](https://yidika-lab.github.io/registry/). It holds one small file per package, `packages/<name>.toml`, with the source repository and, for each release, its commit and SHA-256:

```toml
source = "https://github.com/user/greet"

[[release]]
version = "1.2.0"
commit = "4f1c..."
hash = "sha256-9a0e..."
```

The packages themselves stay on GitHub; the registry only says which commit is which version, and which hash to expect. Its CI accepts a pull request when the entry changes one package, the author owns the source repository, past releases are unchanged, and the tag, commit and hash all match. `YIDI_REGISTRY` points `yidi` at another registry (a URL or a folder) for a mirror or a private one.
