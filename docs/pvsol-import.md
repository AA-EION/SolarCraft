# Import a PV*SOL project

From **Open**, choose a `.pvprj` and a location for a new `.solarcraft` copy. The source archive is read locally. On Windows, choose a destination folder. SolarCraft uses the original basename and adds a suffix if the name is already taken. The CLI call `import_pvsol` takes `source`. Omit `file` to preview. To create a project, supply a new `.solarcraft` file. In MCP these paths are relative to the workspace.

The importer accepts the observed ZIP formats 15, 20, 23 and 24 with embedded `Project.xml`, `DB.json`, and a recognized three-channel WBV. It pins equipment revisions from the archive, imports module areas and their saved orientation, and reads every wiring block. Saved MPPT assignments determine active module counts. A genuinely unwired area remains available for editing and contributes zero generation.

- **Series and optimizers.** Series-connected segments share one current. Supported per-module buck optimizers use their saved voltage, current and power limits and efficiency curve. The desktop wiring editors display these groups and preserve them when applying other changes. Advanced groups can be edited through `set_stringing`. Enter verified DC cable ratings manually for these groups.
- **Legacy island designs.** An identical set of battery clusters on one AC bus becomes an equivalent battery bank. SolarCraft imports the battery and converter ratings, the generator where selected, and available appliance load intervals. It reconstructs supported appliance and generated-load shapes on its reference calendar. Import warnings describe the assumptions for initial SOC, battery dispatch, generator switching and load shedding.
- **Grid designs.** SolarCraft imports a recognized single embedded load profile at its original interval, or the hourly base-load ledger when available. Review the import warnings for settings that require separate migration.

Run the imported design and open **Results → Saved PV*SOL comparison**, or call `compare_pvsol` with the imported `file`. The saved reference and the SolarCraft run are separate. SolarCraft compares annual energy and whatever monthly or 8,760-row reference channels the file provides. The results show the signed kWh and percentage difference, mean absolute and root-mean-square hourly error, and whether the current run uses the imported input hash.

| Reference | Use |
| --- | --- |
| SimResults.xml | First project-total PV DC and inverter AC groups. Annual and monthly energy are stored in watt-seconds. An empty model marker means no qualifying saved aggregate run. |
| core.results.3600 | Recognized hourly PV DC and inverter AC channels. The time column ends each consecutive local-standard-time interval. Signed AC includes standby in the archived series. |
| OecForeCast.json | The signed hourly PV generation ledger is the AC reference when the simulator's own hourly export is absent. Available hourly series supply missing annual and monthly totals. |
| WBV | Embedded temperature, global horizontal radiation and wind. SolarCraft uses a 365-day local standard-time reference calendar and midpoint solar position. It applies Erbs decomposition when saved diffuse readings are absent. With saved DHI it reverses the source spectral correction before deriving DNI with the midpoint solar position and limits nonphysical values near the horizon. |

The comparison covers PV generation. Import and replay still have gaps for 3D scenes and shading, roof and photo calibration, active solar tracking, grid-connected storage, EV and thermal controls, adaptive time steps, tariff details, equipment protection and vendor-specific models. SolarCraft applies its normal electrical validation to imported designs. If validation blocks a run, the saved annual totals remain available in the reference.
