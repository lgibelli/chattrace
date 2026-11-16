# Security

ChatTrace Forensic handles sensitive material, so please report security problems privately:
use GitHub's **Report a vulnerability** button on the repository's *Security* tab, rather than
opening a public issue.

Of particular interest: anything that makes ChatTrace write to the evidence it reads, accepts
an update without a valid signature, lets content from a chat execute or load remote resources
in the viewer or the HTML report, or produces an evidence package that verifies although it was
altered.

Never attach real chat data, evidence packages or Teams caches to an issue. Reproduce problems
with the demo data (`chattrace-cli view --demo`) or with a synthetic case when possible.
