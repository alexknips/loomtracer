# Security policy

## Reporting a vulnerability

Report a vulnerability privately through GitHub. Do not open a public issue
or pull request for it.

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**
   ([direct link](https://github.com/alexknips/loomtracer/security/advisories/new)).
3. Fill in the advisory form.

The report is visible only to you and the maintainer until a fix is
released.

## What to include

- What is affected: the command, MCP tool, REST route or plugin script, and
  the version (`hivemind --version`) or commit.
- What an attacker can do with it: read, change or lose ledger data, act as
  another actor, or reach another tenant's decisions.
- Steps to reproduce, or a proof of concept small enough to run against a
  scratch ledger (`hivemind quickstart` makes one).
- A fix or mitigation, if you have one.

Leave real credentials, API keys and ledger contents that are not yours out
of the report.
