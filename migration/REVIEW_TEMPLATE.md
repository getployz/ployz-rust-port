# Package review handoff

## Inputs

- Package packet: `<path>`
- Oracle package: `<path>`
- Direct callers: `<paths>`
- Crate commit: `<full SHA>`
- Diff command: `git diff <base>...<commit> -- <owned path>`

## Parity review

Use a fresh read-only agent. Account for every behavior-contract and
traceability row. Report missing or changed behavior, errors, ordering, timing,
formats, platform behavior, and observable limitations. Do not require matching
Go files, functions, pointer mechanics, or dependency APIs.

## Rust review

Use a different fresh read-only agent. Check that the implementation follows
the selected dependencies' idiomatic design, uses only approved dependencies,
and handles ownership, errors, async work, blocking calls, cancellation,
platform configuration, safety, and maintenance cleanly.

## Finding format

```text
ID: <P01 or R01>
Priority: <blocking or non-blocking>
Location: <file:line>
Contract or rule: <packet row or documented rule>
Evidence: <concrete failure>
Required correction: <observable outcome, not prescribed implementation>
```

The original implementor fixes findings. Each reviewer rechecks its own
findings and reports `clean` only when no actionable finding remains.

