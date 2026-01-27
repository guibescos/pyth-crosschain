# Pyth Crosschain

This repository acts as a monorepo for the various components that make up
Pyth protocols.

Within this monorepo you will find the following subprojects:

## Table of Contents

- [Target Chains](#target-chains)
- [Hermes](#hermes)
- [Fortuna](#fortuna)
- [Local Development](#local-development)
  - [Quick start](#quick-start)
  - [Toolchain](#toolchain)
  - [Common commands](#common-commands)
  - [Rust services](#rust-services)
  - [Pull requests](#pull-requests)
  - [Releases](#releases)
  - [Typescript Monorepo](#typescript-monorepo)
    - [Building a new web app, JS / TS library or CLI tool](#building-a-new-web-app-js--ts-library-or-cli-tool)
- [Audit / Feature Status](#audit--feature-status)

## Target Chains

> [target_chains](./target_chains/)

This directory contains on-chain contracts and SDKs for all of the various
blockchain runtimes that Pyth supports. Each subdirectory corresponds to a
blockchain runtime. Inside each subdirectory, there are subfolders for
contracts and SDKs.

## Hermes

> [hermes](./apps/hermes/)

Hermes is an off-chain service which constantly observes Pythnet and the
Wormhole network watching for price updates emitted from the Pyth contract. It
exposes all observed attestations via a public API over HTTPS/WSS which can be
consumed by client-side applications that wish to use Pyth pricing data.

The [`price_service/client`](./price_service/client/) directory provides an SDK for interacting with Hermes.
However, most users will interact with the price service via a chain-specific SDK

For a guide on utilising this service in your project, see the chain-specific SDK
and [examples](https://github.com/pyth-network/pyth-examples/tree/main/price_feeds) for your blockchain runtime in the `target_chains` directory.

## Fortuna

> [fortuna](./apps/fortuna/)

Fortuna is an off-chain service which can be used by [Entropy](https://pyth.network/entropy) providers.

## Local Development

### Quick start

1. Install the toolchain below (Node/pnpm, plus chain-specific tools if needed).
2. Ensure your Node version matches `.nvmrc` (currently `v24.0.0`) using your preferred manager:
   - `nvm use`
   - `mise x -- node --version` (or `mise x -- pnpm <command>` for any pnpm command)
   - `nix develop` (if you use nix)
3. Enable the pnpm version from `package.json` (`pnpm@10.28.0`) via Corepack:
   - `corepack enable`
   - `corepack prepare pnpm@10.28.0 --activate`
4. Install dependencies:
   - `pnpm install`

### Toolchain

Keep the tool list concise and only install what you need for the areas you work on:

- [Node.js](https://nodejs.org/en) — match `.nvmrc` (`v24.0.0`). Use [nvm](https://github.com/nvm-sh/nvm) or [mise](https://mise.jdx.dev/), or enter a nix shell with `nix develop` (pair with `direnv allow` if you use direnv).
- [pnpm](https://pnpm.io/) — match `package.json` (`pnpm@10.28.0`) via [Corepack](https://nodejs.org/api/corepack.html).
- [Foundry](https://book.getfoundry.sh/getting-started/installation) for Ethereum contract development (`forge`).
- [Solana CLI](https://solana.com/docs/intro/installation) for Solana programs.
  - After installing, please run `solana keygen new` to generate a local private key.
- [Anchor](https://www.anchor-lang.com/docs/installation) for Solana program development.
- [Pre-commit](https://pre-commit.com/) is used to automatically format and lint the repository.
  - After installing, please run `pre-commit install` in the root of the repo to configure the checks to run on each git commit.
- [Rust](https://www.rust-lang.org/tools/install) for Rust services and SDKs.

### Common commands

All JS/TS packages live in a turborepo. From the repo root:

- `pnpm turbo start:dev`: Start dev servers in parallel.
- `pnpm turbo test`: Run unit/integration tests, linting, and format checks across packages.
- `pnpm turbo fix`: Auto-fix and format where possible.
- `pnpm turbo start:prod`: Build and run production servers.

You can pass any `turbo run` option after `--`, for example:

```
pnpm turbo test -- --concurrency 2
```

### Rust services

Some services and SDKs are Rust-based. Typical commands from the relevant crate directory:

- `cargo build`
- `cargo test`

### Pull requests

Use the [Conventional Commits](https://www.conventionalcommits.org) format for your commit messages and PR titles.
In the PR description, please include a summary of the changes and any relevant context. Also, please make sure
to update the package versions following the [Semantic Versioning](https://semver.org/) rules.

See also: [Code guidelines](doc/code-guidelines.md)

### Releases

The repository has several CI workflows that automatically release new versions of the various components when a new Github release is published.
Each component's workflow uses a specific tag format including the component name and version number (e.g., Fortuna uses the tag `fortuna-vX.Y.Z`).
The general process for creating a new release is:

1. Update the version number of the component in the repo, e.g., in `package.json` or `Cargo.toml` or wherever. Please follow [Semantic Versioning](https://semver.org/) for package versions.
2. Submit a PR with the changes and merge them in to main.
3. Create a new release on github. Configure the release to create a new tag when published. Set the tag name and version for the component you wish to release -- see the [Releases](https://github.com/pyth-network/pyth-crosschain/releases) page to identify the relevant tag.
4. Publish the release. This step will automatically trigger a Github Action to build the package and release it. This step will e.g., publish packages to NPM, or build and push docker images.
   - Note that when publishing a public package, you should prune the auto-generated Github release notes to only include changes relevant to the release. Otherwise, the changelog will include commits from unrelated projects in the monorepo since the previous release.

Note that all javascript packages are released together using a tag of the form `pyth-js-v<number>`. (The `number` is arbitrary.)
If you have a javascript package that shouldn't be published, simply add `"private": "true"` to the `package.json` file
and it will be excluded from the publishing workflow. If you are creating a new public javascript package, you should add
the following config option to `package.json`:

```
  "publishConfig": {
    "access": "public"
  },
```

### Typescript Monorepo

All of the typescript / javascript packages in this repository are part of a
[turborepo](https://turbo.build/repo/docs) monorepo.

#### Building a new web app, JS / TS library or CLI tool

To quickly get started, from the root of this repo, you can run the following:

1. `pnpm create-pyth-package`
2. Answer the prompts
3. Once the script is done, you will have your new webb app, library or CLI tool bootstrapped with all the current best practices.

If you'd like to read more about the best practices, checkout this best practices doc:
[🔗 Creating a new JavaScript / TypeScript Package](./doc/js-code-guidelines.md)

## Audit / Feature Status

⚠ **This software is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
implied. See the License for the specific language governing permissions and limitations under the License.** Or plainly
spoken - this is a very complex piece of software which targets a bleeding-edge, experimental smart contract runtime.
Mistakes happen, and no matter how hard you try and whether you pay someone to audit it, it may eat your tokens, set
your printer on fire or startle your cat. Cryptocurrencies are a high-risk investment, no matter how fancy.
