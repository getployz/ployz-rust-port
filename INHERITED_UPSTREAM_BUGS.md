# Inherited upstream bugs

This ledger records confirmed flaws in the frozen Go oracle that the Rust port intentionally preserves. It prevents parity work from silently becoming feature or bug-fix work.

## Recording flow

1. Reproduce the suspected bug against the pinned Go tree without modifying it. Capture the exact command, input, output, platform, and Go source location.
2. Have both fresh-context reviewers confirm that the behavior comes from the Go oracle and is not a Rust translation defect, test defect, or unsupported-environment result. Escalate reviewer disagreement to a human.
3. Add a numbered entry below before the fixer acts. Record the Rust counterpart and a parity test that demonstrates the same observable behavior.
4. The fixer preserves the behavior and verifies the parity test. Do not weaken, skip, or mark the test ignored.
5. Repair only in a separate, explicitly authorized change after the parity baseline. Link that change from the entry without rewriting the historical evidence.

## Confirmed entries

None. No inherited product bug was confirmed during the three-file trial for issue #2.

The timing-sensitive upstream test failure observed while freezing the oracle remains documented in `UPSTREAM_ORACLE.md`. It is not classified here as a product bug without a reproducible behavioral defect that the Rust port must retain.

## Entry format

Each confirmed entry must contain:

- ID and short title;
- status and confirmation date;
- Go source and test locations;
- exact Go reproduction and observed behavior;
- Rust counterpart and parity-test location;
- reviewer confirmation and human decision, when required;
- upstream issue or fix reference, if one exists;
- any later, separately authorized Ployz fix.
