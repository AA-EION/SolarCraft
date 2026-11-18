# Cable protection checks

Select a rule set per circuit. The circuit editor shows each measured or entered value with its comparison threshold. The Planner report includes the result. Missing ratings remain **Incomplete** and keep the affected circuit from being applied to the simulation.

## NEC 2023 PV DC

The ordinary method for PV DC source/output circuits uses 1.25 times the parallel STC short-circuit current for the maximum circuit current. The unadjusted conductor ampacity and the minimum protective-device current use another factor of 1.25. The corrected ampacity must carry the maximum circuit current. A linked MPPT gets its STC current from the module and string design. When one cable runs to each string, sizing uses the highest string current. For an unlinked run, supply the STC current per conductor path.

Enter the insulation-table ampacity, installation correction factor, and terminal-limited ampacity separately. The tool also checks the entered equipment limit, maximum DC circuit voltage, device voltage rating and fault interruption rating. It applies the ordinary method from NEC 690.8/690.9 with voltage and interruption checks. The checks use the exact entered device size, with no next-standard-size, 100%-rated assembly or alternative maximum-current exception. [NFPA 70](https://www.nfpa.org/codes-and-standards/nfpa-70-standard-development/70) · [University of Maryland Extension](https://www.extension.umd.edu/resource/working-solar-wiring-and-fusing-eb-2023-0676)

## IEC 60364-4-43

The overload checks use Ib ≤ In ≤ Iz and I₂ ≤ 1.45 Iz. The short-circuit checks compare the device's breaking capacity to the available fault current and its let-through energy to (k × conductor area)² in the adiabatic range. Enter I₂, conductor k for the chosen insulation and material, and let-through energy from the device data at the relevant fault duty. The checks also compare the entered circuit and device voltage and AC/DC rating. [IEC 60364-4-43:2023](https://webstore.iec.ch/en/publication/28432) · [Schneider Electric overload checks](https://www.electrical-installation.org/enwiki/Practical_values_for_a_protective_scheme) · [Schneider Electric short-circuit withstand](https://www.electrical-installation.org/enwiki/Verification_of_the_withstand_capabilities_of_cables_under_short-circuit_conditions)

The checks cover the named comparisons. Fault current calculations, fault clearing time at minimum fault current, grounding, selectivity, and local adoption and exceptions require their own design inputs and review.
