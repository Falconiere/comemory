# Release signing keys

This directory is where the **public** half of the minisign keypair belongs —
the one that would sign `sha256.sum`. It is **not committed yet**, and no
release carries a signature, so the verification recipe below cannot succeed
today; it is what to run once signing is actually enabled. The private key (`comemory.key`)
lives in 1Password and is loaded into the release-finalize workflow
runner via the `MINISIGN_KEY` + `MINISIGN_PASSPHRASE` GitHub Actions
secrets — never committed.

## One-time setup (maintainer)

```bash
# 1. Generate the keypair (do this once, on a secure machine).
minisign -G -p keys/comemory.pub -s /tmp/comemory.key \
  -c "comemory release signing <YOUR-EMAIL>"
#    Set that address before running. -c is accepted here even though `minisign
#    -G`'s usage line omits it (minisign.c parses `c:` globally and
#    ACTION_GENERATE passes it to generate()); it becomes the key files'
#    untrusted comment. Without it the key gets minisign's generic default.

# 2. Move the private key to 1Password, delete /tmp/comemory.key.

# 3. On the repo, set two GitHub Actions secrets:
#      MINISIGN_KEY         — contents of comemory.key (the secret half)
#      MINISIGN_PASSPHRASE  — the passphrase you used in step 1
#    gh secret set MINISIGN_KEY < comemory.key
#    gh secret set MINISIGN_PASSPHRASE

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
