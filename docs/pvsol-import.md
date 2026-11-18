# Import a PV*SOL project

From **Open**, choose a `.pvprj` and a location for a new `.solarcraft` copy. The source archive is read locally. On Windows, choose a destination folder. SolarCraft uses the original basename and adds a suffix if the name is already taken. The CLI call `import_pvsol` takes `source`. Omit `file` to preview. To create a project, supply a new `.solarcraft` file. In MCP these paths are relative to the workspace.

The initial importer accepts format 23 and 24 on-grid ZIP projects with embedded `Project.xml`, `DB.json`, and a recognized three-channel WBV. It pins the exact equipment revisions from the archive, imports module areas and their saved orientation, and treats wired inverter counts and MPPT groups as authoritative for installed modules. It imports the recognized single embedded load profile at its original interval, with an hourly base-load fallback from the saved ledger. Supported global loss and grid settings are carried across. Other elements and approximations are listed when reviewing the import.

Run the imported design and open **Results → Saved PV*SOL comparison**, or call `compare_pvsol` with the imported `file`. The saved reference and the SolarCraft run are separate. SolarCraft compares annual energy and whatever monthly or 8,760-row reference channels the file provides. The results show the signed kWh and percentage difference, mean absolute and root-mean-square hourly error, and whether the current run uses the imported input hash.

| Reference | Use |
| --- | --- |
| SimResults.xml | First project-total PV DC and inverter AC groups. Annual and monthly energy are stored in watt-seconds. An empty model marker means no qualifying saved aggregate run. |
| core.results.3600 | Recognized hourly PV DC and inverter AC channels. The time column ends each consecutive local-standard-time interval. Signed AC includes standby in the archived series. |
| OecForeCast.json | The signed hourly PV generation ledger is the AC reference when the simulator's own hourly export is absent. Where aggregates are absent, the annual reference is the sum of those signed hours. |
| WBV | Embedded temperature, global horizontal radiation and wind. SolarCraft uses a 365-day local standard-time reference calendar and midpoint solar position. It applies Erbs decomposition when saved diffuse readings are absent. With saved DHI it reverses the source spectral correction before deriving DNI with the midpoint solar position and limits nonphysical values near the horizon. |

This is a scoped PV replay. It preserves source identity, equipment and wiring, weather and available reference results. PV*SOL legacy formats, off-grid projects, 3D scene and shading migration, detailed protection and transformer import, unsupported string topologies, adaptive time steps, complete tariff and load-device behavior, and Hottel/Hofmann and other vendor-specific models require separate mapping. A saved project with a horizon mask or active thermal system is still useful for comparison, with those differences recorded.
