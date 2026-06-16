# Release signing keys

This directory holds the **public** half of the minisign keypair used to
sign `SHA256SUMS` for every release. The private key (`comemory.key`)
lives in 1Password and is loaded into the release-finalize workflow
runner via the `MINISIGN_KEY` + `MINISIGN_PASSPHRASE` GitHub Actions
secrets — never committed.

## One-time setup (maintainer)

```bash
# 1. Generate the keypair (do this once, on a secure machine).
minisign -G -p keys/comemory.pub -s /tmp/comemory.key \
  -c "comemory release signing <falconieer@gmail.com>"

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
# Download the release artifacts + the minisign signature.
curl -L -O https://github.com/Falconiere/comemory/releases/latest/download/SHA256SUMS
curl -L -O https://github.com/Falconiere/comemory/releases/latest/download/SHA256SUMS.minisig
curl -L -O https://raw.githubusercontent.com/Falconiere/comemory/main/keys/comemory.pub

# Verify the signature.
minisign -V -p comemory.pub -m SHA256SUMS

# Verify the checksums.
sha256sum -c SHA256SUMS
```

## Rotation

Generate a new keypair, re-sign the next release with both, and document
the overlap window in CHANGELOG.md. The `MINISIGN_KEY` secret gets the
new private key, and the old public key stays in git history (under
`keys/comemory.pub` at the previous SHA) so users can still verify older
releases.
