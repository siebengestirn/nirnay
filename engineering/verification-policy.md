# Verification policy

A source file compiling is not proof that its market semantics are correct.
A passing unit test is evidence only for the assertion and fixture it exercises.

Use these states consistently:

- **Proposed**: design exists; implementation may not exist.
- **Implemented**: code exists; no claim of independent verification follows.
- **Verified**: named command and named tests passed on a recorded environment/date;
  scope and limitations are stated.
- **Falsified**: the claim failed its declared falsifier.

No capability becomes `verified` by changing JSON. `cargo xtask claims` requires
references to existing tests and a verification date for every verified claim.
Release evidence should include commit SHA, toolchain, target triple, command,
exit status, test counts and artifact hashes.
