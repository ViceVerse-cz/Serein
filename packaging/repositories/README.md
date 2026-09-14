# Signed package repositories

`build.py` prepares static HTTPS repository trees for apt, dnf/zypper and pacman.
The repository is hosted on GitHub Pages at
`https://viceverse-cz.github.io/Serein/`. Its current package-signing fingerprint is
`CA19DA939E9BCAB500751CE480FE95CAD86141A5`; verify this through an independent
channel before trusting the key. Configure the repository Actions secret
`PACKAGE_SIGNING_KEY` with the ASCII-armored dedicated private signing key, and
Actions variable `PACKAGE_SIGNING_FINGERPRINT` with its full fingerprint. Run the package-repository preparation workflow with
`tag` (an existing Serein release), `channel` (`nightly` or `production`) and
`base_url` (the final HTTPS root). It downloads the distribution-labelled assets,
verifies their release `SHA256SUMS`, signs repositories and uploads the
`signed-package-repositories` artifact. The release workflow calls the same workflow
after publishing a release and deploys the merged tree to GitHub Pages. Manual runs
deploy only when their `deploy` input is enabled; otherwise they leave a reviewable
artifact without changing the site.

Each invocation accepts native packages from **one build distribution and architecture**:

```sh
python3 packaging/repositories/build.py \
  --input release-assets --output site --format deb \
  --channel nightly --distribution ubuntu-26.04 --architecture amd64 \
  --key "$SIGNING_FINGERPRINT" --base-url "$REPOSITORY_BASE_URL"
```

The output is `site/nightly/ubuntu-26.04/amd64/apt/`. Other combinations are
`fedora-44/x86_64/rpm`, `opensuse-tumbleweed/x86_64/rpm` and `arch/x86_64/arch`.
Production uses a separate `production/` tree. Never point an older Ubuntu, another
Fedora release or a downstream Arch snapshot at packages built for a different ABI.
The script verifies the package name and architecture; the workflow/operator must
select artifacts from the matching distribution.

Install build tools on the signing host: Python 3 and GnuPG, plus `apt-utils dpkg`
for Debian, `rpm-sign rpm createrepo_c` for Fedora, or `pacman libarchive` for Arch.
Use a dedicated signing subkey in an ephemeral `GNUPGHOME` with mode 0700. Import
it before invoking the script; signing must work noninteractively (an unlocked agent
or dedicated unencrypted CI subkey stored only as a protected secret). Keep the
offline primary key out of CI. No private keys are written into repository output.
Never expose signing secrets to pull-request jobs or untrusted release artifacts.

apt receives signed `InRelease`/`Release.gpg`, RPM receives signed packages and
`repodata/repomd.xml.asc`, and pacman receives signed packages and database. Signature
verification runs before the output becomes visible. Failures leave no completed
subtree. Existing output is refused: prepare a fresh staging site, retain other
distribution/channel trees from the current site, then atomically switch the host
to the complete snapshot. Retain old snapshots for rollback. This implementation
publishes the supplied release snapshot, not an unbounded history of packages.
apt metadata expires after 30 days to limit stale signed-metadata replay; regenerate
and republish before expiry even when the application version has not changed.

## Automatic setup script

Run the automatic repository setup script to detect your distribution (Ubuntu/Debian, Fedora, openSUSE, Arch Linux), verify the GPG signing key fingerprint, and configure the repository:

```sh
curl -fsSL https://viceverse-cz.github.io/Serein/setup.sh | sh
# Or run from the cloned repository:
# sh packaging/repositories/setup.sh
```

To configure a specific channel or base URL:
```sh
curl -fsSL https://viceverse-cz.github.io/Serein/setup.sh | SEREIN_CHANNEL=production sh
```

After running the script, update your package lists and install `serein` using your distribution's native package manager (`apt`, `dnf`, `zypper`, or `pacman`). Subsequent system updates will automatically update Serein.

## Manual installation from a published repository

Set `BASE` to the configured HTTPS repository root and choose **one** channel.
These examples use nightly (automatic package-manager upgrades remain controlled
by the system administrator). Obtain `EXPECTED_FINGERPRINT` through the owner's
independent trusted channel, not merely from the same downloaded key file.

```sh
BASE=https://YOUR-HOST/YOUR-PATH
CHANNEL=nightly
EXPECTED_FINGERPRINT=YOUR_FULL_PUBLISHED_FINGERPRINT
```

For Ubuntu 26.04 amd64:

```sh
URL="$BASE/$CHANNEL/ubuntu-26.04/amd64/apt"
curl --fail --location "$URL/serein.asc" -o serein.asc
gpg --show-keys --with-subkey-fingerprint serein.asc
# Compare the displayed full fingerprint with EXPECTED_FINGERPRINT before continuing.
sudo install -Dm644 serein.asc /etc/apt/keyrings/serein.asc
printf 'deb [arch=amd64 signed-by=/etc/apt/keyrings/serein.asc] %s ./\n' "$URL" |
  sudo tee /etc/apt/sources.list.d/serein.list
sudo apt update
sudo apt install serein
```

For Fedora 44 x86_64 (use `opensuse-tumbleweed` and zypper commands for openSUSE):

```sh
URL="$BASE/$CHANNEL/fedora-44/x86_64/rpm"
curl --fail --location "$URL/serein.asc" -o serein.asc
gpg --show-keys --with-subkey-fingerprint serein.asc
# Compare the full fingerprint before importing.
sudo rpm --import serein.asc
curl --fail --location "$URL/serein.repo" -o serein.repo
sudo install -m644 serein.repo /etc/yum.repos.d/serein.repo
sudo dnf install serein
```

On openSUSE copy `serein.repo` to `/etc/zypp/repos.d/serein.repo`, then run
`sudo zypper refresh && sudo zypper install serein`. Retain both package and
repository signature checking; never work around a signature failure by disabling it.

For Arch x86_64:

```sh
URL="$BASE/$CHANNEL/arch/x86_64/arch"
curl --fail --location "$URL/serein.asc" -o serein.asc
gpg --show-keys --with-subkey-fingerprint serein.asc
# Compare the full fingerprint before importing and locally trusting it.
sudo pacman-key --add serein.asc
sudo pacman-key --lsign-key "$EXPECTED_FINGERPRINT"
```

Add this stanza to `/etc/pacman.conf`, substituting the actual URL:

```ini
[serein]
SigLevel = Required
Server = https://YOUR-HOST/YOUR-PATH/nightly/arch/x86_64/arch
```

Run `sudo pacman -Syu serein`. Normal `apt upgrade`, `dnf upgrade`, `zypper update`
or `pacman -Syu` subsequently update Serein. Switching channels may require an
explicit package-manager downgrade; do not enable both channels concurrently.
This does not register Serein with Ubuntu/Debian archives, Fedora, AUR or Flathub.

Run `python3 packaging/repositories/test_build.py` for input-boundary checks and
`sh packaging/repositories/smoke-deb.sh` on a Debian/Ubuntu host with the tools above
for an isolated synthetic package, temporary signing key, apt index verification
and tampered-metadata rejection. Neither check installs or launches the application.

Tool contracts: [apt-secure](https://manpages.debian.org/trixie/apt/apt-secure.8.en.html),
[rpmsign](https://rpm.org/docs/6.1.x/man/rpmsign.1),
[repo-add](https://man.archlinux.org/man/repo-add.8).
