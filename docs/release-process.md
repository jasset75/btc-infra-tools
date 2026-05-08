# Release Process

This document describes the operational release flow for `belter`.

## Scope

Use this process for every published version of the CLI.

Important current state as of 2026-03-31:
- `CHANGELOG.md` already contains `## [0.1.0] - 2026-03-10`.
- `ROADMAP.md` defines `0.1.1` as the release being cut.
- `Cargo.toml` workspace version must be set to `0.1.1` before publication.
- Git tags are not present yet in this repository.

For this repository, the next release to cut is `0.1.1`.
Before tagging it, align `CHANGELOG.md`, `Cargo.toml`, and the release tag on that exact version.

## Release Decision

Before cutting a release:

1. Identify the target version from `ROADMAP.md`.
2. Verify that every item in the target release section is implemented and validated.
3. Confirm that operator-facing docs match the shipped behavior.
4. Confirm the semantic version bump type:
   - `patch`: fixes and low-risk operator-visible improvements,
   - `minor`: new backward-compatible command behavior or command surface expansion,
   - `major`: breaking CLI/config/output changes.

Release readiness is not "code merged".
Release readiness is "implemented, validated, documented, and versioned coherently".

## Release Checklist

### 1. Validate the scoped work

- Review the target section in `ROADMAP.md`.
- Make sure every included item is actually complete.
- Validate real behavior on the target node when the release depends on host/runtime integration.
- If a feature was implemented but not originally planned, decide explicitly whether it belongs in this release.

Do not release based only on local dry-runs when the feature touches:
- `launchd`,
- `podman`,
- `podman machine`,
- HTTP readiness semantics,
- local env-file loading,
- host-specific paths or permissions.

### 2. Move release content into `CHANGELOG.md`

- Move completed roadmap items into `CHANGELOG.md` under `Unreleased`.
- Rewrite roadmap bullets into release notes phrased as shipped behavior, not planned work.
- Group entries under `Added`, `Changed`, `Fixed`, or `Removed` as appropriate.
- Remove the released items from the corresponding roadmap section or mark the section as complete if that is the chosen convention.

The changelog is the source of truth for what shipped.
The roadmap is the source of truth for what is still planned.

### 3. Align semantic version everywhere

Update the release version consistently:

- `Cargo.toml` `workspace.package.version`
- any release-specific docs that mention the next version explicitly
- `CHANGELOG.md` heading:
  - move `Unreleased` content into `## [x.y.z] - YYYY-MM-DD`
  - recreate an empty `Unreleased` section above it

`belter --version` is driven by the Cargo package version, so the workspace version must match the intended shipped version.

### 4. Run quality gates

Run the standard local gates:

```bash
just check
just clippy
just test
just install
```

Minimum expectation:
- the workspace compiles,
- lint passes with current warnings policy,
- test suites pass,
- the installed binary prints the expected version with `belter --version`.

### 5. Perform release smoke validation

At least one smoke pass should confirm the shipped operator path for the release scope.

Examples:
- if the release changes `service bring-up`, exercise `belter service bring-up mempool`,
- if the release changes status semantics, verify both text and `--json` output,
- if the release changes config/env resolution, validate with a realistic `.env` or `env_file` setup.

For runtime-integrated releases, validate on the real target node, not only on a development laptop.

### 6. Commit, tag, and publish

Suggested sequence:

```bash
git add CHANGELOG.md Cargo.toml Cargo.lock README.md ROADMAP.md docs/release-process.md
git commit -m "release: x.y.z"
git tag -a vX.Y.Z -m "Release x.y.z"
git push origin main --follow-tags
```

Publishing the commit is not enough.
The release tag must exist on `origin`.

Operational consequence:
- `just install-latest-stable` on operator nodes fetches tags from `origin` and resolves the highest `v*` tag after fetch.
- If the release tag was created locally but not pushed to `origin`, operator nodes will fail with "No release tags found after fetching from origin." or will stay pinned to an older stable tag.

Minimum post-publish verification:

```bash
git ls-remote --tags origin
```

Confirm that `refs/tags/vX.Y.Z` is present before telling operators to install the latest stable release.

If release artifacts or notes are published elsewhere, generate them from the finalized changelog after the tag is pushed.

### 7. Prepare the next iteration

After tagging:

- confirm `ROADMAP.md` points at the next target version,
- keep `CHANGELOG.md` with a fresh `Unreleased` section,
- if needed, bump to the next development version immediately after the release.

## Additional Things To Check

Beyond roadmap validation, changelog migration, and semantic versioning, also check:

- Git consistency: no missing or duplicate release tag for the target version.
- Remote publish consistency: the release tag exists on `origin`, not only in the local repository.
- Date consistency: release heading date should match the actual publication date.
- Documentation consistency: `README.md` and [belter-command-reference.md](belter-command-reference.md) should not describe shipped commands as scaffold behavior unless that is an intentional release limitation.
- Config compatibility: note any config migration or new required env vars in the release notes.
- JSON contract stability: if machine-readable output changes, call it out explicitly as a breaking or operator-relevant change.
- Installation path: verify the installed `belter` being invoked is the one built from this workspace.
- Rollback awareness: know whether operators can safely reinstall the previous binary and reuse the same config.

## Recommended Rule For This Repository

Given the current repository state, use this rule:

- Cut the next release as `0.1.1`.
- Set `Cargo.toml` `workspace.package.version = "0.1.1"` before publishing.
- Finalize the `Unreleased` section as `## [0.1.1] - 2026-03-31` or the actual release date.
- Create tag `v0.1.1` exactly once.

Do not publish a release whose changelog version, Cargo version, and git tag disagree.
