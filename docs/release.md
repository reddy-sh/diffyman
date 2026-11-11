# Releasing diffyman

Every release is built and published by one workflow,
[`.github/workflows/release.yml`](../.github/workflows/release.yml). This page has the one-time setup
the owner does before the first release and the steps to cut each release. The first release is
0.1.0, and nothing has been published yet.

## What a release contains

| Where | What | Built by the job |
|---|---|---|
| crates.io | `diffyman-model`, `diffyman-detect`, `diffyman-adapter-tree`, `diffyman-algo`, `diffyman-adapter-binary`, `diffyman`, `diffyman-render`, `diffyman-cli`: every crate without `publish = false`, each published after the crates it depends on (`python3 scripts/release/versions.py crates` prints the order) | `publish-crates` (`cargo publish` builds from the tagged source) |
| PyPI | `diffyman`: abi3 wheels (CPython 3.11+) for macOS arm64 and x86_64, manylinux_2_28 x86_64 and aarch64, and Windows x64, plus the sdist | `python-wheels`, `python-sdist` (`twine check --strict` in `python-check`) |
| npm | `@diffyman/node` and its prebuilt addons `@diffyman/node-darwin-arm64`, `-darwin-x64`, `-linux-x64-gnu`, `-linux-arm64-gnu` and `-win32-x64-msvc` (installed as optional dependencies, so npm fetches only the one for the platform it installs on); `@diffyman/wasm`; `@diffyman/react` | `node-addons`, `npm-packages` |
| Maven Central | `sh.reddy.diffyman:diffyman`: one jar with the JNI library for macOS aarch64 and x86_64, Linux x86_64 and aarch64, and Windows x86_64, plus the sources and javadoc jars; `publish-maven` signs those very files with GPG | `java-natives`, `java` |
| Workflow artifacts only | The CLI for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (both static) and `x86_64-pc-windows-msvc`, each a `.tar.gz` or `.zip` with the binary, its README and the license texts, plus a CycloneDX SBOM; SBOMs of the binding crates; `SHA256SUMS` over every file of the run | `cli`, `sbom`, `checksums` |

The workflow never creates a GitHub release. The CLI archives are workflow artifacts (kept 30
days); attach them to a GitHub release yourself if you want one.

Every binary is built with `--remap-path-prefix` for the home folder, the Rust installation and
the checkout, and with the linker settings that keep paths out (the PDB file name on Windows, an
`@rpath` install name for macOS libraries). `scripts/release/local_paths.py check` fails the job
if a build-machine path is still left in a binary, archive, wheel, jar or npm package. The CLI is
built with `cargo auditable`, so its dependency list can be read back from the binary
(`cargo audit bin`). The Linux Node.js addons and JNI libraries are linked against glibc 2.28,
with Zig through cargo-zigbuild, like the manylinux_2_28 wheels, and
`scripts/release/check-glibc.sh` checks it.

`@diffyman/wasm` is built as [ADR 0008](adr/0008-wasm-target.md) decides
(`scripts/release/build_wasm.py`): `cargo build` with the release profile, then `wasm-bindgen
--remove-name-section --remove-producers-section`, without wasm-pack and without wasm-opt.
wasm-bindgen-cli is built from source with `cargo install --locked` at the version of the
`wasm-bindgen` crate in `Cargo.lock` (`scripts/release/versions.py lock-version wasm-bindgen`),
so the job downloads no prebuilt binary. The script fails if a `name` or `producers` section is
left and prints the module's sizes. Version 0.1.0, built as the release job builds it (with the
path-remapping RUSTFLAGS) on a Mac with Apple silicon on 2026-11-10: `diffyman_wasm_bg.wasm` is
2,858,266 bytes raw, 879,589 with gzip -9 and 617,130 with brotli 11, with only the
`target_features` custom section left (the same module with its name and producers sections,
as wasm-pack leaves them: 3,327,980 / 932,450 / 653,496). The JavaScript glue is the same
either way, and so is the document it returns.

## Safety rails

- **Triggers.** The workflow runs only when started by hand (`workflow_dispatch`) or when a tag
  `vX.Y.Z` is pushed. It never runs on a branch push or a pull request.
- **Dry run by default.** The `dry_run` input defaults to true. A tag push is always a dry run.
- **Publish jobs.** `publish-crates`, `publish-pypi`, `publish-npm` and `publish-maven` run only
  in a run started by hand, and each waits for an approval of the `release` environment (required
  reviewers). In a dry run they stop before the upload: `cargo publish --dry-run` for every crate
  in dependency order, the checksum check of the wheels and the sdist, `npm publish --dry-run` for
  every package, and the unsigned Maven Central bundle. They upload only when `dry_run` is false.
- **What is uploaded is what was built.** `publish-pypi`, `publish-npm` and `publish-maven`
  download this run's artifacts and check them against its `SHA256SUMS`
  (`scripts/release/verify_sums.py`): every file must be listed with the same SHA-256, and
  every listed file must be there. Nothing is rebuilt in a publish job. The Maven job signs and
  uploads the jars and POM of the `java` job byte for byte (`scripts/release/maven_bundle.py`),
  the same files `check_jar.py` checked and the attestations cover.
- **No unpinned tool next to an OIDC token.** `twine check --strict` runs in `python-check`, a
  job without `id-token`; `publish-pypi` installs nothing and uploads with the pinned
  `pypa/gh-action-pypi-publish`. `publish-npm` sets up exactly Node.js 24.22.2 and stops
  unless `node --version` and `npm --version` are exactly `v24.22.2` and `11.19.1` (the npm
  it comes with); it installs no other tool.
- **Maven Central, a second gate.** The upload uses the Central Portal's `USER_MANAGED`
  publishing type (the Maven plugin's `autoPublish` false): the job waits until the portal has
  validated the deployment, and nothing reaches Maven Central until you press **Publish** on it
  in the portal ([Cutting a release](#cutting-a-release), step 5).
- **The tag.** The `plan` job stops a run that would publish unless it runs on the tag
  `vX.Y.Z` and every manifest has version `X.Y.Z` (`scripts/release/versions.py check`).
- **Attestations.** Build provenance attestations (`actions/attest-build-provenance`, from
  `SHA256SUMS`) are created only in a run that publishes, after the same approval: they are public
  records, and a dry run leaves none.
- **npm `private`.** Every package.json in the repository and in the staged packages says
  `"private": true`. Only the approved publish job runs `npm pkg delete private`, then
  `npm publish --provenance`.
- **Maven.** The workflow never runs `mvn deploy`. Run by hand, `mvn deploy` publishes nothing
  (`maven.deploy.skip`), and with `-P release` it, too, stops at a validated deployment
  (`autoPublish` false in the POM).
- **Workflow hygiene.** The token is read-only by default. Only the jobs that use OIDC get
  `id-token: write`: attest, crates.io, PyPI and npm. Every action is pinned by commit, and the
  workflow uses no caches. Values reach `run:` scripts only through `env:`. The workflow passes
  `actionlint` (with shellcheck) and `zizmor --persona pedantic` without findings.
- **CI.** The `release packaging (dry run)` job of `ci.yml` runs the packaging steps that need no
  credentials on every pull request and push: `versions.py check`, `publish-crates.sh --dry-run`,
  `build_wasm.py`, the npm staging with `npm pack --dry-run`, the jar with `check_jar.py`, the
  unsigned Maven bundle, and `local_paths.py check` on all of them.
- **Reruns.** A publish job can be run again after a failure. Crates and npm packages whose
  version is already on the registry are skipped, so `publish-crates` resumes with the first
  crate that is missing; PyPI skips files it already has. A Maven Central deployment is
  published entirely or not at all; when the version is already on Maven Central the job
  uploads nothing, and if an earlier run's deployment is still waiting in the portal, drop one
  of the two.

## One-time setup

Do these steps in this order, before the first run of the workflow. Create the GitHub environment
first: if a job names an environment that does not exist, GitHub creates it without protection
rules.

### 1. The GitHub `release` environment

1. In **Settings → Environments**, create a **New environment** named `release`.
2. **Required reviewers:** add yourself (and anyone else who may approve a release). Leave
   **Prevent self-review** off while you are the only reviewer.
3. Turn off **Allow administrators to bypass configured protection rules**.
4. **Deployment branches and tags:** choose **Selected branches and tags** and add the branch
   `main` (for dry runs) and the tag pattern `v*.*.*`. The workflow itself refuses to publish
   from anything but the release tag.
5. **Environment secrets** (never repository secrets). Add them as you reach the steps below:

   | Secret | Needed for |
   |---|---|
   | `CARGO_REGISTRY_TOKEN` | The first crates.io release only; delete it afterwards |
   | `NPM_TOKEN` | The first npm release only; delete it afterwards |
   | `MAVEN_CENTRAL_USERNAME`, `MAVEN_CENTRAL_PASSWORD` | Every Maven Central release (a Central Portal user token) |
   | `MAVEN_GPG_PRIVATE_KEY`, `MAVEN_GPG_PASSPHRASE` | Every Maven Central release (the signing key) |

   PyPI needs no secret.
6. In **Settings → Rules → Rulesets**, add a tag ruleset for `refs/tags/v*` that allows only you to
   create, update or delete those tags.

### 2. crates.io

1. Sign in at <https://crates.io> with the GitHub account that owns `reddy-sh/diffyman` and verify
   your email address (crates.io requires it for publishing).
2. **First release:** trusted publishing can only be added to a crate that already exists, so the
   first upload uses a token. In **Account Settings → API Tokens**, create a token with the scopes
   `publish-new` and `publish-update`, limited to the crates `diffyman` and `diffyman-*`, with a
   short expiry (a day is plenty). Store it as the environment secret `CARGO_REGISTRY_TOKEN`.

   **Timing of the first release.** crates.io lets an account create only a few new crates at
   once and then one more every 10 minutes (new versions of existing crates have a far larger
   allowance). The first release creates all eight, so crates.io refuses an upload partway with
   HTTP 429 and the time to try again. `publish-crates.sh --publish` then waits until that
   time (10 minutes when the answer names none) and uploads the same crate again, up to eight
   times per crate. Expect `publish-crates` to take about half an hour for 0.1.0; the job
   allows 90 minutes. Nothing needs doing meanwhile. If the job stops anyway (a timeout, a
   cancelled run), run it again: the crates already on crates.io are skipped, and it resumes
   with the first one that is missing. Later releases only add versions and need no wait;
   their trusted-publishing token is valid for 30 minutes, which is ample.
3. **After the first release:** on each published crate, open **Settings → Trusted Publishing**
   and add a GitHub publisher with repository owner `reddy-sh`, repository `diffyman`, workflow
   `release.yml` and environment `release`. Then delete the `CARGO_REGISTRY_TOKEN` secret and
   revoke the token. Later releases get a short-lived token through
   `rust-lang/crates-io-auth-action`.

### 3. PyPI

1. Create an account at <https://pypi.org> with two-factor authentication.
2. Under **Your projects → Publishing**, add a **pending publisher**. Use PyPI project name
   `diffyman`, owner `reddy-sh`, repository `diffyman`, workflow `release.yml` and environment
   `release`. The first approved run creates the project, and no token is ever stored.

### 4. npm

1. Create an account at <https://www.npmjs.com> with two-factor authentication, and create the
   organization `diffyman` (free, public packages), which owns the `@diffyman` scope.
2. **First release:** npm adds a trusted publisher only to a package that exists. Create a
   granular access token with read and write access to the packages of the `diffyman`
   organization, with a short expiry. Store it as the environment secret `NPM_TOKEN`.
3. **After the first release:** for each of the eight packages, open **Settings → Trusted
   publishing** and add GitHub Actions with user `reddy-sh`, repository `diffyman`, workflow
   `release.yml` and environment `release`. Set **Publishing access** to require two-factor
   authentication and disallow tokens. Then delete the `NPM_TOKEN` secret and the token. The
   publish job uses npm 11.19.1 (from Node.js 24.22.2, both pinned exactly; trusted publishing
   needs npm 11.5.1 or later), and
   `--provenance` links each version to the workflow run that built it.

### 5. Maven Central

1. Create an account at <https://central.sonatype.com>.
2. **Namespace:** under **Namespaces**, add `sh.reddy.diffyman`. The portal shows a verification
   key. Add it as a DNS TXT record on `reddy.sh` (at the host name the portal gives), wait until
   the namespace shows as verified, then remove the record if the portal says it is no longer
   needed.
3. **Token:** under **Account → Generate User Token**, create a token. Its username and password go
   into the environment secrets `MAVEN_CENTRAL_USERNAME` and `MAVEN_CENTRAL_PASSWORD`;
   `scripts/release/maven_bundle.py upload` sends them to the portal's Publisher API. The upload
   only creates a deployment that waits for you: publishing it stays a click in the portal.
4. **Signing key:** create a key that is used only for releases, and publish its public half:

   ```sh
   gpg --quick-generate-key "Sankara Reddy Telukutla <sankara@reddy.sh>" ed25519 sign 2y
   gpg --list-secret-keys --keyid-format long          # the key ID
   gpg --keyserver keyserver.ubuntu.com --send-keys KEYID
   gpg --keyserver keys.openpgp.org --send-keys KEYID   # then confirm the email it sends
   gpg --armor --export-secret-keys KEYID              # paste into MAVEN_GPG_PRIVATE_KEY
   gpg --output diffyman-release-revoke.asc --gen-revoke KEYID
   ```

   Store the passphrase as `MAVEN_GPG_PASSPHRASE`. Keep the revocation certificate and a backup
   of the key offline, not in the repository. The publish job imports the key into a keyring of
   its own and removes it when the step ends.

### 6. GitHub Pages (the documentation site)

[`.github/workflows/docs.yml`](../.github/workflows/docs.yml) builds the site on every push and
pull request, and deploys it to <https://reddy-sh.github.io/diffyman> only when you run it by
hand on `main` with **deploy** ticked. Protect the environment it deploys to **before** you turn
Pages on, so that no deployment can happen without your approval:

1. In **Settings → Environments**, create a **New environment** named `github-pages` (GitHub
   would create it without protection rules when Pages is first enabled).
2. **Required reviewers:** add yourself. Turn off **Allow administrators to bypass configured
   protection rules**.
3. **Deployment branches and tags:** choose **Selected branches and tags** and add only `main`.
4. Only then, in **Settings → Pages**, set **Source** to **GitHub Actions**.

A deploy run builds the site without restoring any cache, then waits for your approval of
`github-pages`.

## Cutting a release

1. **Set the version** on a branch:

   ```sh
   scripts/release/set-version.sh 0.1.0
   ```

   The script writes the version into the Cargo workspace and both `Cargo.lock` files, the
   package.json and package-lock.json of `bindings/node`, `bindings/wasm/react` and
   `examples/amplify-react`, the Maven POM, `docs/api/openapi.json` and the jar names in the Java
   docs. The Python and WASM packages and the CLI's `--version` take it from Cargo. It then checks
   that every manifest agrees, and CI runs the same check
   (`python3 scripts/release/versions.py check`). Update `docs/STATUS.md`, then run the local
   checks:

   ```sh
   python3 scripts/release/versions.py check --expect 0.1.0
   sh scripts/release/publish-crates.sh --dry-run --allow-dirty   # every crate, in order
   ```

   Merge the change once CI is green, including its `release packaging (dry run)` job.

2. **Dry run.** In **Actions → Release → Run workflow**, choose `main`, enter the version and leave
   **Dry run** ticked. Every artifact is built and checked. When the four publish jobs ask for the
   `release` environment, approve them: each checks the files against `SHA256SUMS` and runs its
   dry run, and none uploads. Download the artifacts
   and look at them: the CLI archives and their SBOMs, `release-checksums`, the wheels, the npm
   folders and the jars. Then run the [install check](#install-check) on them.

3. **Tag** the merged commit and push the tag:

   ```sh
   git tag -s v0.1.0 -m "diffyman 0.1.0"
   git push origin v0.1.0
   ```

   The tag push builds everything once more, as a dry run without publish jobs.

4. **Release.** In **Actions → Release → Run workflow**, choose the tag `v0.1.0` under **Use
   workflow from**, enter `0.1.0` and untick **Dry run**. The `plan` job checks the tag and the
   versions, then the run builds everything again. Approve the `release` environment for
   `attest build provenance` and the four publish jobs. `publish-crates` takes about half an hour
   for the first release ([crates.io](#2-cratesio)).

5. **Publish the Maven Central deployment** (the second gate). `publish-maven` ends with a
   deployment named `diffyman-0.1.0` in the state **Validated**, not published. Open
   <https://central.sonatype.com/publishing/deployments>, check that it holds
   `sh/reddy/diffyman/diffyman/0.1.0/` with the jar, the sources and javadoc jars and the POM,
   each with its `.asc` signature and checksums, then press **Publish**. **Drop** discards it
   instead, and nothing reaches Maven Central. Publishing takes a few minutes in the portal and
   up to about 30 minutes until the files are on Maven Central.

6. **Check** what was published:

   ```sh
   cargo info diffyman@0.1.0
   pip download --no-deps diffyman==0.1.0
   npm view @diffyman/node@0.1.0 optionalDependencies
   gh attestation verify diffyman-0.1.0-x86_64-unknown-linux-musl.tar.gz --repo reddy-sh/diffyman
   ```

   Search for `sh.reddy.diffyman` on <https://central.sonatype.com>. For the first release,
   finish the trusted publishing steps above and delete the two tokens.

A version can never be uploaded again to any of these registries. To withdraw a broken release,
yank the crates (`cargo yank --version 0.1.0 CRATE`), yank the release on PyPI, and deprecate the
npm versions (`npm deprecate`). Maven Central artifacts cannot be removed. Then release a new patch
version.

## Install check

`scripts/release/install-check.sh` installs the release files in clean containers the way the
[quickstarts](quickstart/cli.md) say and runs every example of each quickstart with them: one
container per language, which sees only its files (read-only) and a script that
`scripts/release/quickstart_script.py` makes from the quickstart page. The script saves the
page's files, runs its commands, checks their exit status and compares what each prints with
the page. One command per language, or every language at once:

```sh
scripts/release/install-check.sh FILES python     # or cli, rust, node, browser, java, c
scripts/release/install-check.sh FILES            # all seven, then a table of the results
```

`FILES` is a folder of release files for the Docker engine's architecture (searched
recursively). Each container is timed from its start to its end; the images are pulled before.

| Language | Image | Files | Installed with |
|---|---|---|---|
| `cli` | `debian:trixie-slim` | `diffyman-VERSION-ARCH-unknown-linux-musl.tar.gz` | `tar -xzf`, on `PATH`; jq and patch from apt for the JSON and patch examples |
| `rust` | `rust:1.99.1` | the eight `.crate` files | `cargo install --locked diffyman-cli` from a local copy of crates.io: the eight crates and their dependencies at the versions of the packaged `Cargo.lock` (copied on the host first); no network |
| `python` | `python:3.12-slim` | `diffyman-VERSION-cp311-abi3-manylinux_2_28_ARCH.whl` | the quickstart's `pip install`; no network |
| `node` | `node:22-slim` | `diffyman-node-VERSION.tgz`, `diffyman-node-linux-ARCH-gnu-VERSION.tgz` | the quickstart's `npm install` with the two files; npm offline, so the other platforms' optional packages are skipped at once; no network |
| `browser` | `node:22-slim` | `diffyman-wasm-VERSION.tgz`, `diffyman-react-VERSION.tgz` | the quickstart's `npm install` with the two files (React and Vite from npm), `npx vite build`; the `render` examples need a DOM, so instead the built page's `.wasm` asset diffs the page's two inputs in Node.js |
| `java` | `eclipse-temurin:17-jre` | `diffyman-VERSION.jar` | the quickstart's class path, with Jackson from `~/.m2`; a JRE has no compiler, so the examples are compiled on the host for Java 17; no network |
| `c` | `debian:trixie-slim` | `libdiffyman_ffi.so` (not a release file) | the quickstart's `cc` command, with gcc and jq from apt |

The host needs Docker and Python 3, plus `cargo` for `rust` and a JDK and Jackson in `~/.m2`
(`mvn -f bindings/java/pom.xml verify` downloads it) for `java`. `--keep` keeps the work
folder (each container's log is there), and `--remove-images` removes the images the run
pulled.

**After a dry run**, download the run's artifacts (`gh run download RUN_ID --dir release`) and
pack the npm folders:

```sh
npm pack release/release-npm/node release/release-npm/node-linux-arm64-gnu \
  release/release-npm/wasm release/release-npm/react --pack-destination release
```

The workflow uploads no `.crate` files and no C library. `publish-crates.sh --dry-run` leaves
the crates in `target/package/tmp-crate/`; build the C library for Linux as below.

**Before a release**, the same files can be built on a Mac with Apple silicon for Linux arm64
(Docker's architecture there), with Zig as the release workflow uses it:

```sh
uv venv --python 3.12 "$HOME/zig"
uv pip install --python "$HOME/zig" --require-hashes -r scripts/release/zig-requirements.txt
rustup target add aarch64-unknown-linux-musl aarch64-unknown-linux-gnu
export PATH="$HOME/zig/bin:$PATH"
export RUSTFLAGS="$(python3 scripts/release/local_paths.py rustflags --target aarch64-unknown-linux-gnu)"
mkdir -p files addons natives/linux-aarch64 natives/macos-aarch64

cargo zigbuild --release --locked -p diffyman-cli --target aarch64-unknown-linux-musl
python3 scripts/release/package_cli.py --version 0.1.0 --target aarch64-unknown-linux-musl \
  --binary target/aarch64-unknown-linux-musl/release/diffyman --out files
cargo zigbuild --release --locked -p diffyman-ffi -p diffyman-java --target aarch64-unknown-linux-gnu.2.28
cp target/aarch64-unknown-linux-gnu/release/libdiffyman_ffi.so files/
maturin build --release --locked --zig --target aarch64-unknown-linux-gnu \
  --compatibility manylinux_2_28 -m bindings/python/Cargo.toml --out files   # maturin 1.16.0

npm ci --prefix bindings/node
npm ci --prefix bindings/wasm/react
bindings/node/node_modules/.bin/napi build --cwd bindings/node -o "$PWD/addons" --platform --release \
  --no-js --no-dts-header --dts native.d.ts -x --target aarch64-unknown-linux-gnu.2.28 -- --locked
cargo install --locked wasm-bindgen-cli --version "$(python3 scripts/release/versions.py lock-version wasm-bindgen)"
python3 scripts/release/build_wasm.py --out bindings/wasm/pkg-web
npm run build --prefix bindings/wasm/react
node bindings/node/scripts/stage-npm-packages.mjs --version 0.1.0 --out npm --addons addons \
  --wasm-pkg bindings/wasm/pkg-web --react bindings/wasm/react --allow-missing
npm pack npm/node npm/node-linux-arm64-gnu npm/wasm npm/react --pack-destination files

cp target/aarch64-unknown-linux-gnu/release/libdiffyman_java.so natives/linux-aarch64/
cargo build --release --locked -p diffyman-java
cp target/release/libdiffyman_java.dylib natives/macos-aarch64/
mvn -f bindings/java/pom.xml -P release verify -Dgpg.skip=true \
  "-Ddiffyman.release.natives=$PWD/natives" "-Ddiffyman.native.dir=$PWD/natives/macos-aarch64"
cp bindings/java/target/diffyman-0.1.0.jar files/

sh scripts/release/publish-crates.sh --dry-run --allow-dirty
cp target/package/tmp-crate/*-0.1.0.crate files/

scripts/release/install-check.sh files
```

Results of version 0.1.0 on a Mac with Apple silicon (Docker 29.6.2, linux/arm64), files
built as above, 2026-11-09. Each time runs from the container's start to its end, the
quickstart's install command and every example included:

| Language | Files | Time | Examples |
|---|---|---|---|
| `cli` | `diffyman-0.1.0-aarch64-unknown-linux-musl.tar.gz` | 3.3 s (apt-get included) | 19 of 19 checks |
| `rust` | the eight crates, `cargo install --locked diffyman-cli` | 16.4 s (the build included) | 15 checks; the 2 jq examples and their output left out |
| `python` | `diffyman-0.1.0-cp311-abi3-manylinux_2_28_aarch64.whl` | 1.3 s | 11 of 11 |
| `node` | `diffyman-node-0.1.0.tgz`, `diffyman-node-linux-arm64-gnu-0.1.0.tgz` | 0.6 s | 11 of 11 |
| `browser` | `diffyman-wasm-0.1.0.tgz`, `diffyman-react-0.1.0.tgz` | 8.3 s (React and Vite from npm included) | 7 checks and the built page's engine; the 3 `render` examples left out |
| `java` | `diffyman-0.1.0.jar` | 0.6 s | 8 of 8 |
| `c` | `libdiffyman_ffi.so` | 6.6 s (apt-get with gcc included) | 10 of 10 |

## The scripts

| Script | What it does |
|---|---|
| `scripts/release/set-version.sh VERSION` | Sets the version everywhere, then checks it |
| `scripts/release/versions.py check [--expect V]` / `set V` / `crates` / `lock-version NAME` | The version check (CI runs it), the setter, the crates.io publish order, and a package's version in `Cargo.lock` (for wasm-bindgen-cli) |
| `scripts/release/publish-crates.sh --dry-run` / `--publish V` | `cargo publish --dry-run` for all crates in one call, or the uploads in order (an existing version is skipped; a 429 rate limit is waited out and the crate uploaded again) |
| `scripts/release/build_wasm.py --out DIR` | `@diffyman/wasm` as ADR 0008 builds it: cargo, the pinned wasm-bindgen without name and producers sections, `finish-package.mjs`, the sizes |
| `scripts/release/verify_sums.py --sums FILE --root DIR ARTIFACT...` | Downloaded artifacts equal to `SHA256SUMS`, no file missing or extra (every publish job that uploads files) |
| `scripts/release/maven_bundle.py bundle` / `upload` | The Central Portal bundle of the built jars and POM (signed with `--sign`), and its upload as a deployment that waits for a manual Publish |
| `scripts/release/local_paths.py rustflags` / `check` | The path-remapping RUSTFLAGS, and the check for build-machine paths in binaries and packages |
| `scripts/release/package_cli.py` | The reproducible CLI archive (`SOURCE_DATE_EPOCH`) |
| `scripts/release/sbom.py` | CycloneDX SBOMs from cargo-cyclonedx, with the checkout's path replaced |
| `bindings/node/scripts/stage-npm-packages.mjs` | Lays out and checks the eight npm packages and writes their publish order (the only staging step; its tests are in `bindings/node/test/package.test.mjs`) |
| `scripts/release/check_jar.py` | The jar has every platform's library, the license texts, sources and javadoc |
| `scripts/release/check-glibc.sh` | A Linux library needs glibc 2.28 at most |
| `scripts/release/zig-requirements.txt` | cargo-zigbuild and Zig, pinned with hashes |
| `scripts/release/install-check.sh FILES [LANGUAGE...]` | Installs the release files in clean containers and runs every quickstart example with them ([install check](#install-check)) |
| `scripts/release/quickstart_script.py PAGE` | A quickstart page's examples as one POSIX shell script, for the install check |
