# 2. Commercial and licensing decision memo

Draft 2026-11-12. This memo is not legal advice. Items marked **Counsel** need a qualified lawyer in each launch jurisdiction. The analyzed vendor is German, so German and EU law apply directly.

## 1. Reference-material policy

Decided by the owner on 2026-11-12 (D-04). The detailed provenance notes live in the private evidence pack, not in this repository.

1. Behavioural findings about PV\*SOL can inform SolarCraft's mathematics and workflows as far as they are useful: equations, units, defaults, thresholds, sign conventions and file structures. Every implementation is still written from scratch, and published methods are cited wherever they exist.
2. **No vendor data enters SolarCraft:** no catalog rows, numerical model assets (weights, matrices), templates, strings, icons or example projects. The equipment catalog is built by the owner and by users, who enter their own products manually like PV\*SOL users do, and through imports of files that users have rights to.
3. PV\*SOL project import (`.pvprj`, `.wbv`) is an approved goal (D-06). It is developed against project files that the owner or users create with their own licensed copies.
4. Recommended, not blocking: an EU/German IP counsel review of points 1 and 3 before public release (open item L1).

## 2. Licence and business-model options

> **Owner decision (D-01/D-02):** Option B. GPL-3.0-or-later app with paid services; AGPL server; proprietary catalog data. The analysis below is kept for the record.

All options keep the offline local workflow: local catalog items, pinned project snapshots and simulation, with no account, telemetry or network.

| | A. Permissive app + paid hosted catalog | B. GPL-3 app + paid data/support/hosting | C. Proprietary app (compatible deps) | D. Open core / dual licence |
|---|---|---|---|---|
| Code licence | Apache-2.0 (or MIT/Apache dual) for core, shells, CLI, MCP | GPL-3.0-or-later (AGPL for server) | Closed, with an EULA | Core Apache or GPL; premium modules closed, or GPL + commercial licence |
| IngeTrazo code reuse | No, unless the GPL parts run as a separate program (see §4) | Yes, directly | No, unless negotiated | Only in the GPL edition |
| What customers pay for | Verified catalog sync, org libraries, team sharing, support SLA, integrations, hosted compute | Same, plus support contracts | Licence per seat + catalog | Premium modules + catalog |
| Value capture | Data and service. The app is a funnel. | Data and service. The app is a funnel. | Software + data | Split |
| Competitor replication | Anyone can fork the app and bundle their own catalog. The data and curation pipeline are not forkable. | Forks must stay GPL, so no proprietary competitor fork | Hard | Medium |
| Adoption / trust | Highest. Easy enterprise procurement, no copyleft review. | High for individuals. Some enterprise legal friction. | Normal for this market | Medium. Community wary of open-core boundary shifts. |
| Store distribution | Microsoft Store and Mac App Store both fine | Mac App Store terms conflict with GPLv3 when there are third-party copyright holders (VLC 2011 precedent); direct download + notarization works | Fine | Edition-dependent |
| Enterprise procurement | Easy | Legal review of copyleft, though usually acceptable for an end-user app | Standard | Standard |
| Prerequisites | Contributor terms (DCO, or CLA if relicensing is wanted later) | Same | All code owned | **CLA required** for dual licensing; impossible for IngeTrazo-derived code |
| Main risk | A well-funded competitor ships our app with their data | Narrower partner ecosystem (OEM/white-label manufacturers want proprietary embedding) | Must out-execute PV\*SOL/PVsyst on a closed product without community leverage | Complexity and community distrust |

**Recommendation (for owner decision): Option A**, with Apache-2.0 rather than MIT for the explicit patent grant. Keep the brief's "prefer MIT" for *dependencies*. The hosted catalog **service code** goes in the same repo under the same licence (it is a commodity). The **catalog data** is proprietary and licensed per subscription. Use a CLA, or at minimum a DCO plus an assignment for core-team code, so that option D stays open. Why A: the defensible asset is verified, rights-cleared equipment data plus manufacturer relationships, not the simulation code, which pvlib and SAM already provide openly. A permissive app also maximises distribution, OEM/white-label deals with manufacturers and MCP/automation adoption. The cost is that IngeTrazo code cannot sit inside the app (see §4).

Choose **B** instead if the owner ranks "use IngeTrazo code directly" above OEM embedding and Mac App Store distribution. Choose **C** only if seat licensing is the main revenue. In that case, budget for an independent geometry engine anyway.

## 3. Catalog business model

### 3.1 Comparable products and prices (checked 2026-11-12)

| Product | Price | Model | Source |
|---|---|---|---|
| PV\*SOL premium 2027 | €845/yr (extension €802.75); 3-month €285; plus VAT | Named user, Windows only, catalog included | [Valentin shop](https://shop.valentin-software.com/en/collections/kategorie-photovoltaik) |
| PV\*SOL (2D) 2026 | €585/yr | same | same |
| PVsyst | ~CHF 700/yr per professional licence (secondary sources disagree, ~€500–745) | Named user, desktop | [secondary review](https://www.heavengreenenergy.com/blog/pvsyst-price) (the official pricing page returned 404 at check time; **verify**) |
| Aurora Solar | ~$159–259/user/month | SaaS | secondary, [quickestimate](https://quickestimate.co/blog/solar-design-software-pricing); **verify** |
| HelioScope | ~$99–300/user/month | SaaS (Aurora-owned) | same; **verify** |
| OpenSolar | Free core, paid add-ons | SaaS, financed by partners | same |
| SAM (NREL) | Free, BSD-3 | Desktop | [sam.nrel.gov](https://sam.nrel.gov/sam-open-source.html) |
| Meteonorm web app | CHF 320 / 980 / 4,200 per yr | Data subscription | [meteonorm.com/buy](https://meteonorm.com/en/buy) |

Takeaways: European desktop designers pay about €600–900 per seat per year for a bundle of software and catalog. US SaaS seats cost 2–4× more but include CRM and proposal tooling. For SolarCraft, the catalog-plus-service subscription has to clearly beat "free app + manually typed datasheets". A plausible price band is **€15–35 per seat per month**, with team tiers above that.

### 3.2 Unit-economics model (assumptions, year 3 run-rate)

| Assumption | Low | Base | High |
|---|---|---|---|
| Active free users | 3,000 | 12,000 | 40,000 |
| Paid conversion | 5% | 8% | 10% |
| Paid seats | 150 | 960 | 4,000 |
| Blended price per seat/yr (ex VAT) | €240 | €300 | €360 |
| Revenue | €36k | €288k | €1.44M |
| Payment/MoR fees (merchant of record, which handles VAT/GST) | 6% | 5.5% | 5% |
| Variable support per seat/yr | €30 | €25 | €20 |
| **Contribution per seat** | €196 | €258 | €322 |
| Catalog curation (FTE × €70k): onboarding, verification, corrections | 1.0 | 2.0 | 3.5 |
| Manufacturer data licences / weather resale licences | €0 | €20k | €60k |
| Hosting, egress, monitoring (signed bulk diffs keep egress small) | €6k | €15k | €40k |
| Server/billing engineering share | 0.3 FTE | 0.5 FTE | 1 FTE |
| Legal, accounting, insurance | €15k | €25k | €40k |
| **Fixed catalog-business cost/yr** (engineering FTE at €100k) | ≈ €121k | ≈ €250k | ≈ €485k |
| **Break-even paid seats** | ≈ 620 | ≈ 970 | ≈ 1,500 |
| Result at stated seats | −€92k | ≈ break-even | +€800k |

Initial catalog build (one-off, base): about 20k records at 6–12 minutes verification each, plus tooling, which comes to **€90k–180k**. Manufacturer self-service submission with automated range checks moves most of the cost onto suppliers over time. This P&L excludes app development, which is the product investment in [01 §1.3](01-reference-inventory-and-feasibility.md#rough-effort-and-cost-ranges-for-planning-not-a-quote). The catalog business pays back the curation team. It does not fund the app before year 3–4 in the base case.

### 3.3 Options the owner can switch on (independently)

| Option | Who pays | Notes |
|---|---|---|
| Paid verified catalog sync | Designers / installers | Core offer. Projects stay pinned and calculable offline after the subscription ends. |
| Organisation libraries and sharing | Firms | Private collections, roles, audit. Strong willingness to pay among EPCs. |
| Verified data updates / changelog feeds | Firms, banks, IEs | Datasheet citations and version history support bankability. |
| Manufacturer listing / data hosting | Manufacturers | Must never affect ranking or simulation. Disclose sponsorship. |
| Integration services (CRM, ERP, mounting configurators) | Firms | Professional services. Low margin, high stickiness. |
| Support SLA and training | Firms | |
| Hosted batch compute (P90 sweeps, portfolio runs) | Firms | Optional. The same engine runs locally. |

### 3.4 Validation before building the server

- **Interviews (12–20):** 6 residential installers (DE/AT/CH, ES/IT, UK), 4 C&I EPCs, 2 independent engineers, 2 manufacturers' technical marketing teams, 2 distributors. Use a script that covers current tool and price, catalog pain, willingness to pay for verified data, Mac usage, and BIM/CAD needs.
- **Pilot offer:** a free design-partner programme for 10 firms during S1–S2, with a "founding" price of €19/seat/month locked for 2 years.
- **Decision rule:** go with the paid catalog if at least 40% of interviewed firms say they would pay ≥€20/seat/month *and* at least 3 manufacturers agree in writing to supply rights-cleared data. Otherwise, pivot the revenue to support, integrations and OEM.

## 4. IngeTrazo options

> **Owner decision (D-03):** embed IngeTrazo code. Port its geometry/topology to the Rust core under GPL-3.0-or-later with attribution, and run its Python importers headless until ported. Path 1 below, carried out as a port.

**Inspected:** [`ingelibre/ingetrazo`](https://github.com/ingelibre/ingetrazo) at commit `ffc7187bbaf8222ea5f5f19e33f6c4ed81ad4ad5` (tag v0.6.1, 2026-11-08). Licence file: GPL-3.0 text; README and SPDX headers: **GPL-3.0-or-later**.

| Fact | Evidence |
|---|---|
| Language/runtime | Python ≈293k lines + Rust helper crate `ingetrazo_core` (≈4.2k lines, PyO3/numpy/rayon) |
| UI/runtime coupling | PySide6 (Qt 6, LGPL-3). 51 of 80 `core/*.py` import PySide6, including `geometry.py`, `topology.py`, `scene.py` and `mesh.py`. Not usable headless without Qt. |
| Dependencies | PySide6 (LGPL-3), numpy (BSD), openskp @ `291700bc` (MIT, fork), ezdxf (MIT), manifold3d (Apache-2.0), vendored LibreDWG (GPL-3), truststore (MIT) |
| Contributors / rights | Lead author Marco Sumari Tellez (≈450 commits) plus ≥10 other contributors. CONTRIBUTING says contributions are licensed GPL-3.0-or-later. **No CLA**, so a relicence needs agreement from every copyright holder whose code is included. |
| Plugin/extension API | Python plugins in-process; "not stable yet" (`docs/plugins.md`) |
| Automation | MCP "AI bridge" over localhost TCP 127.0.0.1:4763 (`docs/ai-bridge.md`) |
| Performance (self-reported) | Edits in large groups ~0.7 s; push/pull 0.6–1.3 s; ~3 KB per face Python object; "millions of faces … tens of seconds and several GB" (ROADMAP.md, 2026-10-12) |
| Platforms | Linux, Windows, macOS arm64 (unsigned). No Intel macOS, no Windows ARM64 build found. |

| Path | Fit | Licence consequence | Verdict |
|---|---|---|---|
| 1. Integrate or link IngeTrazo code into SolarCraft | Poor technically: a CPython + Qt runtime inside WinUI/SwiftUI shells, non-native UI, performance below utility-scale needs | SolarCraft as a whole becomes GPL-3 (combined work). Option A becomes impossible. | Only under Option B, and even then technically weak |
| 2. Contribute upstream + separate companion | Good. SolarCraft publishes an open exchange format/API. A **SolarCraft bridge plugin for IngeTrazo** (GPL-3, separate repo) lets IngeTrazo users model buildings and send them to SolarCraft over files or MCP. | Communicating at arm's length (files, sockets, command lines) normally keeps programs separate, per the [GPL FAQ on aggregation](https://www.gnu.org/licenses/gpl-faq.en.html#MereAggregation). But intimate data-structure sharing can still create a combined work, so **counsel reviews the interface design**. | **Recommended** in addition to path 4 |
| 3. Negotiated licence from rights holders | Needs every contributor. The code is still Python/Qt, so it does not solve native performance. | Possible in principle, costly to clear | Not recommended now. Revisit if IngeTrazo adopts a CLA. |
| 4. Independent SolarCraft geometry engine (Rust) | Good. Native speed, headless, shared by UI/CLI/MCP/server. Purpose-built for roofs, terrain, obstacles, module fields and shading, which is narrower than a general modeller. | Clean if the team does not copy IngeTrazo code. Use the same MIT/Apache deps (openskp, manifold, ezdxf-equivalent). | **Recommended** core |

IngeTrazo still contributes value without code reuse. It can be a free companion modeller for Linux users and BIM tagging. Its maintainers are potential partners: co-marketing, a shared open `.skp`/IFC test corpus, and upstream fixes to openskp. **Owner decision Q3.** Approver for the boundary design: counsel. Approver for any negotiated licence: IngeTrazo's copyright holders.

## 5. Other licence obligations to plan for

- **LGPL (Qt not used; IfcOpenShell LGPL-3):** dynamic linking plus relinkable distribution. Fine for Options A, B and C. Store builds need a documented way for users to replace the library.
- **MPL-2.0 (UniFFI):** file-level copyleft on modified MPL files only.
- **GPL-3 (LibreDWG):** ship it only as a separate optional helper executable that users download explicitly, under an arm's-length exchange (DXF on stdin/stdout). Counsel confirms. ODA is the paid alternative.
- **Data:** ODbL (OSM/Overture buildings) requires share-alike for derived *databases* but not for produced works (maps, reports). Projects that embed derived building footprints need attribution, and possibly ODbL terms for the embedded data. CC BY 4.0 (NASA POWER per AWS registry; verify on POWER) requires attribution. The EUPL-1.2 notice on the PVGIS docs site covers the software, so the data reuse policy must be **checked** separately.
- **Notices/SBOM:** generate CycloneDX SBOMs per platform artefact, keep a `THIRD_PARTY_NOTICES` file, and offer corresponding source for any copyleft component.

## 6. Unresolved legal questions and approvers

| # | Question | Approver |
|---|---|---|
| L1 | Counsel review of the reference-material policy (§1) before public release (recommended) | Owner + EU IP counsel |
| L2 | Code licence for SolarCraft (A/B/C/D) and contributor terms | Owner |
| L3 | IngeTrazo bridge boundary is not a combined work | Counsel |
| L4 | openskp and LibreDWG reverse-engineered format readers: distribution risk | Counsel |
| L5 | Catalog data licence, offline caching terms and post-cancellation rights | Owner + counsel |
| L6 | Manufacturer data-supply agreement template | Counsel |
| L7 | Using provider data in commercial redistribution: PVGIS, NASA POWER, Overture, Copernicus DEM, national LiDAR | Counsel, per provider (see register) |
| L8 | Trademark wording for interoperability claims ("imports PV\*SOL® projects") | Counsel |
