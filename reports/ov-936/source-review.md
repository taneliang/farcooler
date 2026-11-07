# Source pin reconciliation

Manager supplied immutable fae3f30d0c3d1c475f9dc4463243f293fd88b007 and
accepted Store/policy/atomic mechanisms, explicitly withholding overall notice
integration pending ov-938. Product HEAD at task start equals that pin; the
pre-existing untracked .codex/.cursor files are not product inputs.

Approved ov-935 bdff40f0b2944173c2be5b5fa906dcd2af88f7bf reports schema64.
Actual supplied migrate.rs appends workflow_notices at position65. This is the
expected composition delta, not permission to use schema61 notice history as
cutover evidence. The immutable map and independent source review remain inputs,
not execution or live-data approval.

The actual schema65 catalog is produced by the real registry into a new private
in-memory SQLite fixture, before any cutover entry. catalog-first.log/json records
native Python, exclusive original923 target/FIFO, profile000/jobs6/offline,
source hashes, holder probes, unchanged bytes and reserve/deadline outcome.
87 tables / 201 schema objects / 540 declared columns. Schema object digest:
wf1:9feeae84e856a84252be1ae4d038e3c35122d1808921d9b8754a304e730d3b09.
Counts are inventory crosschecks; typed field and relationship conservation is
independently required and must reject a same-count altered value.

Concrete counterexamples requiring implementation beyond current ownership:

- workflow_ownership::available refuses a historical terminal/line/landing row,
  so ordinary enqueue cannot import legacy work while preserving that evidence.
- The four ownership triggers do not stop task status/wait mutation, check-in
  admission, wake claims, landing jobs or a previously admitted external process.
- workflow_wire_write calls apply_in directly and therefore needs the same
  incarnation/freeze guard as workflow_apply. Guarding only workflow_apply
  would leave a real authoritative bypass.
- Store::init sweeps raw waits even on a current database; raw manifests must
  precede open and transferred/frozen evidence must survive subsequent opens.
- workflow_admit_effect previously used a single CAS, but the new global
  incarnation check must share its writer transaction, not race in a prior read.
- backup.rs remains the existing single DB-file helper. Closed DB/resource
  backup, independent restore and external-effect reconciliation use a separate
  finite private rehearsal seam and are not inferred from that helper.

Runtime bootstrap and native notice admission regions remain reserved to
ov-937/938. No DB-only proof is represented as having joined an external writer.
Exact final file:line anchors and counterexample receipts will be added at freeze.
