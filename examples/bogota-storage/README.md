# Walkthrough: Bogotá house with a hybrid inverter and a battery

The input files are SolarCraft's own illustrative datasheet entries. They describe no real product.

`walkthrough.sh` runs the whole designer workflow with the `solarcraft` CLI:

1. Create the project and set the site, weather, grid connection and consumption.
2. Author equipment offline.
3. Lay out the roof with a chimney.
4. Wire an invalid string and see it refused; then wire a valid one on two MPPTs.
5. Add a DC-coupled battery with a 30 % backup reserve and an outage.
6. Set a time-of-use tariff and the costs.
7. Simulate; inspect the energy balance and the hours behind it; read the cash flow.
8. Save variants and compare them.
9. Write the PDF and DOCX reports.
10. Reopen and rerun offline, then check the results are identical.

```bash
cargo build --release -p solarcraft-cli
```

```bash
examples/bogota-storage/walkthrough.sh /tmp/sc-walk
```

Clear-sky demo weather is used offline. To use a real typical year, pass a saved PVGIS TMY JSON or EPW file as the second argument:

```bash
examples/bogota-storage/walkthrough.sh /tmp/sc-walk tmy_4.711_-74.072.json
```

To get the PVGIS file, run `solarcraft weather fetch` (this sends only the coordinates to PVGIS), or download it from the PVGIS website.

The same steps run as an automated test through the JSON API, which is the same API the desktop apps and MCP server use:

```bash
cargo test --release -p solarcraft-app --test designer_workflow
```
