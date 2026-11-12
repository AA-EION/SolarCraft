# SolarCraft discovery, phase 0

Status: **reviewed**. Owner decisions are recorded in [07-owner-questions.md](07-owner-questions.md) and [ADR 0001](../decisions/0001-foundation.md). Where the analysis below recommended something different (licence, IngeTrazo), the decision overrides it.
Prepared: 2026-11-12.

| # | Deliverable | File |
|---|---|---|
| 1 | Reference inventory, feasibility, discovery plan | [01-reference-inventory-and-feasibility.md](01-reference-inventory-and-feasibility.md) |
| 2 | Commercial and licensing decision memo (incl. IngeTrazo) | [02-commercial-licensing-memo.md](02-commercial-licensing-memo.md) |
| 3 | PV\*SOL feature matrix, 167 rows (narrative + TSV) | [03-feature-matrix.md](03-feature-matrix.md), [feature-matrix.tsv](feature-matrix.tsv) |
| 4 | Open-source candidates and algorithm comparison | [04-oss-and-algorithms.md](04-oss-and-algorithms.md) |
| 5 | Platform, stack and interoperability feasibility plan | [05-platform-interop-feasibility.md](05-platform-interop-feasibility.md) |
| 6 | Provider register | [06-provider-register.md](06-provider-register.md), [provider-register.tsv](provider-register.tsv) |
| 7 | Owner decisions, batch 1 | [07-owner-questions.md](07-owner-questions.md) |
| — | Assumptions log (reversible choices made to keep working) | [assumptions.md](assumptions.md) |

## Evidence classes used in every document

| Code | Meaning | Strength |
|---|---|---|
| `DOC` | Publicly documented by the vendor (product page, help, release notes) | Says the feature exists. Says little about how it behaves. |
| `STATIC` | Static behavioural findings from the private reference material | Possible code path in a specific build. Names are hypotheses. |
| `SAVED` | Values in saved project or result files described in the bundle | True for that saved input and version only. The raw files are **not** in the bundle. |
| `RUNTIME` | Confirmed by running the reference application | **No row has this today.** The bundle states the app was never run. |
| `PUB` | Standard or peer-reviewed publication, or an open implementation | Basis for SolarCraft's own implementation. |
| `BRIEF` | Required by the SolarCraft brief, with no PV\*SOL counterpart found (an improvement or a new workflow) | Scope commitment, not parity. |
| `HYP` | Hypothesis or inference awaiting a test | Must not ship as a claim. |

## Handling rules for the private reference bundle

- The bundle stays outside this repository. `.gitignore` blocks the usual paths. Documents here cite private evidence keys. They do not copy listings, tables, weights or templates.
- Equations are written into SolarCraft specs only when a public source (`PUB`) states them. The bundle is used to find behaviour, defaults, thresholds, units and test boundaries.
- The rules for using the bundle are in [02 §1](02-commercial-licensing-memo.md#1-reference-material-policy). Citations live in a private evidence index, keyed `EV-<row id>`.
