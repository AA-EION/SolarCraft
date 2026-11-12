# Assumptions log

Reversible working assumptions I made so that independent work could continue. They are **not** approvals. Each one is revisited when the linked question is answered.

| ID | Assumption | Why | Reverses if | Linked question |
|---|---|---|---|---|
| A-01 | ~~Bundle for behaviour only; importers wait~~ **Closed:** superseded by D-04/D-07 (maths may be used, no vendor data, PV\*SOL import in scope) | Provenance concerns (02 §1) | Counsel clears broader use | Q7 |
| A-02 | **Confirmed (D-11):** Parity baseline = PV\*SOL premium 2026 R4 (analysed) through R8 (documented). 2027 R1 is tracked as separate rows. | 2027 R1 shipped 2026-11-11 | Owner sets a different baseline | Q9 |
| A-03 | Discovery docs live in this repo without bundle citations (D-08). | Brief asks for a monorepo with docs | Owner wants them private/elsewhere | Q8 |
| A-04 | ~~No licence file~~ **Closed:** GPL-3.0-or-later chosen (D-01/D-02) | | | |
| A-05 | Spike code lives under `spikes/` in the GPL repo and is deleted or promoted after the reports | Avoid locking a licence in early | — | Q2 |
| A-06 | ~~EN+DE, DACH~~ **Closed:** all markets incl. Colombia/LatAm; locales en, de, es, pt-BR (D-05) | PV\*SOL's home market; the bundle's locale evidence | Owner chooses other segments | Q1 |
| A-07 | Rust core, WinUI 3/C#, SwiftUI shells as the working hypothesis | §05 analysis | Spikes fail criteria | foundation review |
| A-08 | Internal units are SI. Timestamps are UTC, labelled as interval start, with explicit interval length. | Avoids DST and label ambiguity | — | — |
