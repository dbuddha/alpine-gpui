# AEP 0564: Engineering skill assurance registration

- Status: Accepted scope registration
- Requirement: [#564](https://github.com/dbuddha/alpine-gpui/issues/564)
- Capability: [#28](https://github.com/dbuddha/alpine-gpui/issues/28)
- Corrective defect: [#584](https://github.com/dbuddha/alpine-gpui/issues/584)
- Implementation: [#565](https://github.com/dbuddha/alpine-gpui/issues/565)
- Behavioral evaluation: [#566](https://github.com/dbuddha/alpine-gpui/issues/566)

## Context and scope

The owner-approved seven-skill system extends the installation and authority
boundaries established by [AEP 0184](0184-evidence-first-github-operations.md).
Its implementation was registered in Issues but omitted from the assurance
registry. The hierarchy workflow therefore rejected closure of Task #565 with
`parent #564 has no registered assurance claims`.

This records the existing approved scope and its implemented structural controls.
It does not change a shipping API, dependency, architecture, required gate, or
authorization policy. Requirement #184 retains its accepted historical evidence.
The [engineering skill contract](../operations/alpine-engineering-skills.md) and
[evaluation protocol](../quality/engineering-skill-evaluation.md) remain canonical
for routing and behavioral acceptance.

## Atomic claims

- **AEP-0564-C01:** The canonical seven-skill manifest drives repository-owned
  metadata validation and installation. Invalid inventory entries fail closed;
  installation is idempotent and preserves foreign paths.
- **AEP-0564-C02:** With evidence enforcement enabled, closing implementation
  Task #565 passes the registered-parent precondition without closing Requirement
  #564 while evaluation #566 remains open. Removing the parent registration is
  rejected before child completion is considered, without a remote mutation.

## Evidence and limitations

`scripts/test-agent-skills.sh` exercises manifest, metadata, installation, check,
removal, and foreign-path controls. `scripts/test-hierarchy.sh` invokes the
production reconciler against fixture GitHub responses and the real registry for
#565/#564/#566, plus a temporary missing-registration registry. The test checks
both the parent-child query and absence of close/reopen mutations. An unrelated
unregistered approved parent must still fail.

These are deterministic integration claims, not accepted agent-effectiveness
results. Registry validation establishes traceability, not that a test ran or a
research conclusion is true. Required exact-head and exact-main CI and an
applicable live reconciliation provide separate execution receipts.

No behavioral improvement, independent forward-evaluation result, tamper-verifier
implementation, E3 skill effectiveness, renderer advantage, or M1-M5 milestone
completion follows from this registration. Skill-effectiveness evaluation remains
under #566 and its accepted protocol. Renderer advantage and M1-M5 acceptance
remain under their own approved requirements and qualification experiments; #566
does not own or close those product and renderer obligations. Register precise
executable claims with the responsible work, not placeholder artifacts or an
assertion that structural checks prove behavior.

Kani, TLA+, Miri, GPU timing, and physical display tests do not prove these shell
integration claims. They are not required evidence for this non-shipping registry
correction; existing repository assurance gates remain unchanged.

## Closure and recovery

Requirement #564 closes only when all required children and its full acceptance
contract pass, including independent behavioral evaluation. Keep the effectiveness
experiment open until that evidence exists. A successful #565 reconciliation is
not permission to close the requirement or to promote its evidence level.

If registration, parent identity, approval, or child state is missing or invalid,
retain the failure and correct the canonical record. Never disable evidence
enforcement, reopen completed #184 to mask this omission, or discard the failed
hierarchy run. This registration does not fix unrelated hierarchy limitations.
