# Release signing keys

This directory is where the **public** half of the minisign keypair belongs —
the one that would sign `sha256.sum`. It is **not committed yet**, and no
release carries a signature, so the verification recipe below cannot succeed
today; it is what to run once signing is actually enabled. The private key
(`comemory.key`) lives in 1Password and is loaded into the release-finalize
workflow runner via the `MINISIGN_KEY` + `MINISIGN_PASSPHRASE` GitHub Actions
secrets — never committed.

### What still stands between here and a signed release

The setup below is two of the three remaining items:

1. `release-finalize.yml`'s `ubuntu-22.04` runner ships no `minisign`, so
   `scripts/sign-release.sh` soft-skips before it ever looks at the key. Needs
   an install step.
2. `MINISIGN_KEY` + `MINISIGN_PASSPHRASE` unset.
3. `keys/comemory.pub` not committed.

Three earlier blockers are fixed. Worth recording, because they explain a whole
class of missing output: that workflow had **never run** — 0 runs across every
release since it was added — because `release.yml`'s host job publishes the
release with `secrets.GITHUB_TOKEN`, and GitHub does not dispatch workflows from
events a `GITHUB_TOKEN` created. It also downloaded artifacts with
`actions/download-artifact` (which cannot see a sibling run's artifacts) and ran
without `contents: write`. It now triggers on the `Release` workflow completing,
takes the archive off the release, and has the permission it needs. The same
root cause is why every release body up to and including v0.16.0 is
cargo-dist's auto-generated one rather than the curated notes.

## One-time setup (maintainer)

```bash
# 1. Generate the keypair (do this once, on a secure machine).
minisign -G -p keys/comemory.pub -s /tmp/comemory.key \
  -c "comemory release signing <YOUR-EMAIL>"
#    Set that address before running. -c is accepted here even though `minisign
#    -G`'s usage line omits it (minisign.c parses `c:` globally and
#    ACTION_GENERATE passes it to generate()); it becomes the key files'
#    untrusted comment. Without it the key gets minisign's generic default.

# 2. Set the two GitHub Actions secrets, while the private key still exists —
#    before step 3, which deletes the file the first command reads.
#      MINISIGN_KEY         — contents of /tmp/comemory.key (the secret half)
#      MINISIGN_PASSPHRASE  — the passphrase you used in step 1
gh secret set MINISIGN_KEY < /tmp/comemory.key
#    Read the passphrase and pipe it: `--body "$pass"` would put the secret in
#    argv (visible in `ps`) and in shell history, which is the exposure
#    scripts/sign-release.sh deliberately avoids. Run this line on its own —
#    `gh secret set NAME` with no value reads stdin, so pasting the whole block
#    would feed `read` the lines below it.
read -rsp 'passphrase: ' pass </dev/tty && printf '%s' "$pass" \
  | gh secret set MINISIGN_PASSPHRASE && unset pass

# 3. Move the private key to 1Password, then delete /tmp/comemory.key.

# 4. Commit keys/comemory.pub.
git add keys/comemory.pub
git commit -m "chore(release): commit minisign public key"
```

## Verifying a release (user)

```bash
base=https://github.com/Falconiere/comemory/releases/latest/download
archive=comemory-x86_64-unknown-linux-gnu.tar.xz   # swap for your platform

# Checksums — this part works today.
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL -O "$base/$archive"
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL -O "$base/sha256.sum"
awk -v a="$archive" '$2 == "*" a || $2 == a' sha256.sum > line.sum \
  && [ -s line.sum ] \
  && sha256sum -c line.sum   # macOS: shasum -a 256 -c line.sum

# Signature — only once a release carries one and comemory.pub is committed.
curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL -O "$base/sha256.sum.minisig" \
  && curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL -O \
       https://raw.githubusercontent.com/Falconiere/comemory/main/keys/comemory.pub \
  && minisign -V -p comemory.pub -m sha256.sum
```

## Rotation

Generate a new keypair, re-sign the next release with both, and document
the overlap window in CHANGELOG.md. The `MINISIGN_KEY` secret gets the
new private key, and the old public key stays in git history (under
`keys/comemory.pub` at the previous SHA) so users can still verify older
releases.
