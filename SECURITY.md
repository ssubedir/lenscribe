# Security policy

## Supported versions

Security fixes target the current default branch and the latest stable release. Older releases may require an upgrade; separate maintenance branches and backports are not currently provided. Reports affecting released versions or current source are welcome.

## Report a vulnerability

Use GitHub's [private vulnerability reporting form](https://github.com/ssubedir/lenscribe/security/advisories/new) for Lenscribe. Do not disclose vulnerability details in a public issue or pull request.

If private reporting is unavailable, open an issue asking @ssubedir to enable it. Include only the request for a private reporting channel; wait for one before sharing the vulnerability or proof of concept.

Include:

- The affected Lenscribe version or source commit and operating system.
- A description of the issue, its impact, and any conditions needed to trigger it.
- Reproduction steps or a minimal proof of concept using synthetic data.
- Relevant sanitized logs and, if known, the affected component or a suggested fix.

Maintainers will review the report, ask for clarification where needed, and coordinate a fix and disclosure with you. Response and release timing depend on maintainer availability and the issue's severity. Please keep details private while a fix is being coordinated.

## Data handling

Lenscribe sends original image bytes to the selected model provider when extraction is enabled. Saved API keys are plain text in local settings, and extracted text is stored in image trailers and the local database. The optional search API is unauthenticated and bound to loopback. These behaviors are documented in [Data and privacy](README.md#data-and-privacy) and [architecture](docs/architecture.md).

Keep real API keys, private images, extracted text, and updater signing keys out of reports. Use sample files where possible. Ordinary extraction mistakes and feature requests belong in [public issues](https://github.com/ssubedir/lenscribe/issues); unintended access, disclosure, or changes to data should be reported privately.
