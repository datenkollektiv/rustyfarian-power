# Contributing to rustyfarian-power

Thanks for your interest in contributing!
This project is maintained by the *rustyfarians* (Rust enthusiasts around datenkollektiv)
and is part of the [rustyfarian family](README.md#rustyfarian-family) of embedded-Rust crates
for battery-powered ESP32 field deployments.

All kinds of contributions are welcome — code, documentation, bug reports, ideas, or small cleanups.

---

## 🧰 Prerequisites

- **[`just`](https://github.com/casey/just)** — the task runner behind every build/test/lint command; install it before running any `just …` recipe.
- **Rust + the `esp` toolchain** — install via [`espup`](https://github.com/esp-rs/espup) (or `just setup-toolchain`); the channel is pinned in `rust-toolchain.toml`. Host-only checks run on stable, but building for the device needs the Espressif Xtensa toolchain.
- **ESP-IDF tooling** — only required for device tasks (`just check-all`, `just build-example`, `just flash`); it is fetched automatically on the first ESP-IDF build. Pure host-side work (`just verify`) needs none of it.
- Optional: `cargo-deny` and `cargo-audit` (for `just deny` / `just audit`) — CI installs these automatically.

Run `just doctor` for a one-glance check of your environment.

---

## 🚀 How to Contribute

### 1. Fork & Branch
- Fork the repository
- Create a feature branch from `main`
- Keep changes focused and small where possible

### 2. Make Your Changes
- Read [AGENTS.md](AGENTS.md) first — it is the cross-tool operating guide (project overview, architecture, conventions)
- Follow existing code style; prefer clarity over cleverness
- Keep the hardware-independent core (`config.rs`, `sleep.rs`, `charging.rs`) free of ESP-IDF dependencies so it stays host-testable
- Avoid breaking existing behavior unless discussed

### 3. Validate Before Opening a PR
Run the non-modifying verification suite — this **mirrors the host-side CI gates** (the `fmt`, `clippy`, and `rust` workflows):

```shell
just verify
```

For changes to ESP-IDF-gated code (`esp_*.rs`) or examples, also validate the cross-compile **locally** (these are *not* run in CI, which has no ESP toolchain):

```shell
just check-all
just build-example <example>
```

`just audit` (and the scheduled Audit workflow) scans dependencies for security advisories. Note it generates a `Cargo.lock` if one isn't present — the lockfile is gitignored for this library, so this is expected and not something to commit.

### 4. Open a Pull Request
- Describe **what** you changed and **why**
- If the change is visible or behavioral, mention it explicitly
- If it's cleanup-only, say so clearly

---

## 🧹 "Boy Scout Pass" (Cleanup Changes)

We sometimes refer to a **"Boy Scout pass"**, inspired by the Boy Scout Rule:

> *Always leave the code a little cleaner than you found it.*

A Boy Scout pass means small cleanups, improved readability, or structure — with no intentional
behavior changes. When in doubt, label your change `cleanup`, `refactor`, or
`boy scout pass (no behavior change)`.

---

## 🧪 Testing

- Host-side logic lives behind traits and is unit-tested without hardware — add tests for new core logic
- If your change affects behavior, mention what you tested in the PR (and, for hardware changes, which board you flashed)
- Cleanup-only changes need only a quick sanity check

---

## 📝 Commit Messages

We prefer simple, descriptive commit messages. No need to be overly formal — just be clear.

---

## 💬 Communication & Conduct

- Be respectful and friendly; assume good intent
- Keep discussions technical and constructive
- See our [Code of Conduct](CODE_OF_CONDUCT.md)

This is an open-source hobby project — let's keep it enjoyable.

---

## ❓ Questions or Ideas?

- Open an issue
- Start a discussion
- Or just submit a PR and see what happens 🙂

Thanks for helping make `rustyfarian-power` better!
