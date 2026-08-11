# Dependency requests

The controller stores unresolved capability requests here using
`migration/dependencies/REQUEST_TEMPLATE.md`. One capability has one request,
even when many package packets wait for it.

The research agent owns only its decision file under `migration/dependencies/`.
The controller owns request status and registry updates.
