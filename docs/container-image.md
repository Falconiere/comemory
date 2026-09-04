# Container image

`comemory` publishes a multi-arch image to
**`ghcr.io/falconiere/comemory`** on every `v*` tag, built by
`.github/workflows/release-image.yml` from the `Dockerfile` at the repository
root.

The image is a convenience wrapper around the same binary the release tarballs
carry — it is compiled from the tagged source with `--locked`, so it resolves
the dependency graph in the committed `Cargo.lock`, and the build fails if
`comemory --version` disagrees with the tag that triggered it.

## Tags

| Tag              | Moves | Meaning                                            |
| ---------------- | ----- | -------------------------------------------------- |
| `0.17.0`         | no    | exactly that release                               |
| `latest`         | yes   | newest **stable** release (a `-` marks prerelease) |
| `v17`            | yes   | newest release in the compatible series            |

`vN` follows the semver compatibility boundary: while the major is `0` that is
the **minor** (`0.17.x` → `:v17`, `0.18.x` → `:v18`); from `1.0` on it is the
**major** (`1.x` → `:v1`). The number resets once at `1.0.0`, so `:v1` is newer
than `:v17`.

Prereleases publish only their exact version — they never move `latest` or `vN`.

Every publish prints a digest-pinned ref
(`ghcr.io/falconiere/comemory@sha256:…`) to the job summary. **Pin the digest**
anywhere the image matters; the moving tags are for humans trying things out.

## Architectures

`linux/amd64` and `linux/arm64` — the same two Linux targets `dist.targets` in
`Cargo.toml` already ships tarballs for. Each leg compiles on a native runner
rather than under QEMU: `rusqlite` (bundled SQLite), `sqlite-vec` and `git2`
(vendored libgit2) are all C built from source, and emulating that is slow
enough to be flaky.

## Using it

The image is entrypointed on the binary, so it behaves like the CLI:

```bash
docker run --rm ghcr.io/falconiere/comemory --version
docker run --rm ghcr.io/falconiere/comemory search "retry policy" --json
```

Two paths matter:

- **`/data`** — `COMEMORY_DATA_DIR`, holding the markdown memories and
  `comemory.db`. Declared as a volume; mount it to keep anything.
- **`/work`** — the working directory, where a repository is expected to be
  mounted for `index-code` / `search-code`.

```bash
docker run --rm \
  -v comemory-data:/data \
  -v "$PWD":/work:ro \
  ghcr.io/falconiere/comemory index-code --path /work
```

### Running as another user

The image runs as uid `10001` (`comemory`), a fixed high uid chosen so it
cannot collide with a real account on the host. A bind-mounted data directory
owned by someone else will not be writable; override the user in that case:

```bash
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$HOME/.comemory":/data \
  ghcr.io/falconiere/comemory list
```

A named volume (the first example) avoids the problem entirely and is the
better default.

### Vectors

The image ships no embedder — `comemory` is BYO-vector by design, and that does
not change in a container. Pass vectors with `--vector` / `--vector-stdin` as
usual; `scripts/comemory-embed.sh` is the sample Ollama wrapper and runs on the
host, not in here.

## Building locally

```bash
docker build -t comemory:local .
docker run --rm comemory:local --version
```

`COMEMORY_VERSION` is optional locally — pass it to get CI's assertion that the
binary matches the version you think you built:

```bash
docker build --build-arg COMEMORY_VERSION="$(cargo metadata --no-deps --format-version 1 \
  | jq -r '.packages[0].version')" -t comemory:local .
```

## First publish: package visibility

A GHCR package is **private** when it is first created, and nothing in the
workflow can change that — visibility is an account-level setting on the
package. After the first tag publishes, make it public once, by hand:

> GitHub → your profile → Packages → `comemory` → Package settings → Change
> visibility → Public

Until then `docker pull` needs a `read:packages` login. The setting is sticky:
later publishes to the same package keep whatever visibility it has.

The package links itself to this repository through the
`org.opencontainers.image.source` label the Dockerfile sets, which is also what
makes it inherit the repository's access rules.
