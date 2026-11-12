# 6. Provider register

The source of truth is [`provider-register.tsv`](provider-register.tsv): 31 providers, with a `terms_checked` date per row. Columns: feature rows, provider, tier, public alternative, local fallback, region, resolution, auth, pricing, terms checked, redistribution/caching, quotas, whether user data leaves the machine, expected spend, switch conditions and decision.

## Principles

1. **The default offline workflow needs no provider.** Coordinates are typed by hand. Weather, load profiles, footprints, DEMs and equipment are imported from files. Every remote result is cached *into the project* with provenance (provider, endpoint, query, retrieval time, licence, hash), and only where the terms allow.
2. **Nothing leaves the machine without explicit selection.** Each provider call shows what is sent (usually only coordinates) and to whom. Uploading model files (APS) needs per-action confirmation and owner policy approval.
3. **A free endpoint is not a licence.** A provider becomes a default only once its reuse, caching and commercial terms have been read and recorded in the register.
4. **Paid and private APIs are last-resort adapters**, configured with the user's own key, which is stored in the OS credential store.

## Current defaults (proposed)

| Need | Default | Fallback | Status |
|---|---|---|---|
| TMY weather | PVGIS (PRV-01) | NASA POWER (PRV-02), file import | Data reuse policy still to read: the fetch hit HTTP 429 today |
| Horizon | PVGIS horizon + local DEM ray-march | file import | |
| Basemap | Self-hosted vector tiles from OSM/Overture data (ODbL) | user image | **Not** the OSM tile server: its policy forbids offline/prefetch use (PRV-08) |
| Building footprints | Overture buildings (ODbL) | OSM, state LoD2 | |
| Terrain | Copernicus GLO-30 | national LiDAR | Licence wording to verify |
| Geocoding | User-configurable (Nominatim at ≤1 req/s or self-host) | manual | |
| Equipment seed | CEC lists | manufacturer agreements | |

## Gaps needing paid or partner data for parity

Global satellite TMY at vendor quality (Meteonorm/Solcast/SolarAnywhere: PRV-03..05), high-resolution orthophotos outside open-data countries (PRV-09), Google Solar-style DSMs (PRV-10), Rexplorer-equivalent German meshes (PRV-12; the open LoD2 data PRV-11 may be enough), Europe-wide battery/hybrid equipment data (PRV-23/24), regulated tariff feeds (PRV-20), native DWG write (PRV-25) and native Revit (PRV-26/27). Each is deferred until the owner sets the budget (Q4) and the relevant parity row comes up.

## Rows still to check

Terms for PVGIS data reuse, Solcast, SolarAnywhere, Copernicus DEM, DWD, BDEW and Elexon profiles, ENTSO-E, URDB, CEC lists, the SketchUp SDK and Google Solar caching. These are scheduled for discovery step D2. Each row's `terms_checked` column will hold the date and URL once it is read.
