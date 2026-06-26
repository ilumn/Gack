# Security Policy

Gack shells out to the local `git` executable, so security reports are especially useful when they involve command construction, path handling, terminal escape rendering, or unsafe repository input.

## Reporting

Please report security issues privately to the project maintainer before filing a public issue.

Include:

- Gack version or commit.
- Operating system.
- `git --version`.
- A minimal reproduction when possible.
- Whether the issue requires opening an untrusted repository.

## Scope

In scope:

- Command injection or shell invocation bugs.
- Unsafe handling of file paths from a repository.
- Terminal escape rendering problems.
- Data loss from incorrect Git mutations.

Out of scope:

- Vulnerabilities in Git itself.
- Issues requiring arbitrary local code execution before launching Gack.
