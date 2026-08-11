# Package packets

The controller stores one packet per Go package in this directory using
`migration/PACKAGE_TEMPLATE.md`. Encode nested package paths with hyphens, for
example `internal-machine-network.md`.

Only the controller or assigned packetizer edits a packet. An implementor starts
only when its row in `migration/PACKAGES.tsv` is `ready` and the packet references
all approved dependency decisions.

The packet is the complete handoff to a fresh implementor task. It must not rely
on conversation history.

