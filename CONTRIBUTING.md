# Contributing to Heimdall

Thanks for helping. Bug reports, fixes, and improvements are all welcome.

## Before you start

Heimdall is **source-available, not open source**. It is licensed under the
[Sustainable Use License 1.0](LICENSE).

- **Allowed:**
  - reading, modifying, and running the code for your own internal business,
    personal, or non-commercial purposes;
  - sharing it free of charge for non-commercial purposes.
- **Needs a separate license:** selling it, offering it to others commercially,
  or distributing a commercial version. Contact samhlarsen@proton.me.

## The Contributor License Agreement

Every contributor signs the [Contributor License Agreement](CLA.md) once,
before their first pull request is merged. You keep the copyright in your work.
The agreement gives the project's owner the right to license your contribution
under any terms, including commercial ones, which keeps the whole project under
one owner.

Signing takes one comment. When you open your first pull request, the CLA check
asks you to post this exact sentence on it:

> I have read the CLA Document and I hereby sign the CLA

Your signature is recorded, and the check passes on this and every future pull
request. If it doesn't update, comment `recheck`.

## Making a change

- **Read the spec.** `docs/SPEC.md` is the source of truth for product
  behaviour. Read the relevant section before changing a contract, a limit, or
  a tool schema, and update the spec in the same change.
- **Follow the agent instructions.** `AGENTS.md` lists the hard boundaries and
  conventions. They apply to people too.
- **Build and test** with the commands under
  [Building from source](README.md#building-from-source). A pull request should
  leave all of these passing:

  ```bash
  cargo test --workspace
  cargo clippy --workspace --all-targets -- -D warnings
  (cd apps/desktop && npm test)
  ```

- **Keep each pull request to one change.** Explain in its description why the
  change is needed, not just what it does.

## Reporting a security issue

Please don't open a public issue for a vulnerability. Email
samhlarsen@proton.me instead.
