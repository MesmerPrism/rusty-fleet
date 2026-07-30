# Validation Authority Canary

This disposable documentation-only change verifies that Rusty Fleet's
base-owned pull-request authority runs from `main`, inspects the server-owned
pull-request refs without executing candidate content, and reports the stable
`validation-authority/base-owned` check context.

The canary is not intended for merge.
