// SPDX-License-Identifier: GPL-3.0-or-later
// SolarCraft customer report template (original design).
#let d = json("data.json")
#let s = d.s
#let teal = rgb("#0f5c63")
#let ink = rgb("#1f2937")
#let muted = rgb("#6b7280")
#let rule = rgb("#e5e7eb")

#set document(title: d.title + " — " + d.project_name, author: d.designer, keywords: ("SolarCraft", "photovoltaic"))
#set text(font: "Source Sans 3", size: 10pt, fill: ink, lang: d.lang)
#set page(paper: "a4", margin: (x: 18mm, top: 20mm, bottom: 18mm),
  header: context { if counter(page).get().first() > 1 [
    #set text(size: 8.5pt, fill: muted)
    #d.project_name #h(1fr) #d.title
    #v(-4pt) #line(length: 100%, stroke: 0.5pt + rule)
  ] },
  footer: context [
    #set text(size: 8pt, fill: muted)
    #d.footer #h(1fr) #s.page #counter(page).display() / #counter(page).final().first()
  ])
#set par(justify: false, leading: 0.6em)
#show heading.where(level: 1): it => block(above: 1.4em, below: 0.7em)[#text(size: 14pt, weight: "semibold", fill: teal, it.body)]

#let grid-table(rows, widths: (1fr, auto), header: none, align-cols: (left, right)) = {
  let cells = ()
  if header != none { for h in header { cells.push(text(weight: "semibold", size: 9pt, fill: muted, h)) } }
  for r in rows { for c in r { cells.push(c) } }
  table(columns: widths, stroke: (x, y) => (bottom: if y == 0 and header != none { 0.8pt + teal } else { 0.4pt + rule }),
    inset: (x: 4pt, y: 4.5pt), align: (x, y) => align-cols.at(x, default: right), ..cells)
}

// Cover
#block(width: 100%, inset: (bottom: 6pt))[
  #text(size: 9pt, fill: teal, weight: "semibold", upper("SolarCraft"))
  #v(2pt)
  #text(size: 22pt, weight: "bold", d.title)
  #v(-6pt)
  #text(size: 14pt, fill: muted, d.project_name)
]
#grid(columns: (1fr, 1fr), gutter: 12pt,
  [#text(size: 8.5pt, fill: muted, s.prepared_for) \ #text(weight: "semibold", d.customer.name) \ #d.customer.address],
  align(right)[#text(size: 8.5pt, fill: muted, s.date) \ #d.date \ #d.designer])
#v(8pt)
#grid(columns: (1fr, 1fr, 1fr, 1fr), gutter: 8pt, ..d.kpis.map(k => block(fill: rgb("#f3f7f7"), radius: 4pt, inset: 9pt, width: 100%)[
  #text(size: 8.5pt, fill: muted, k.label) \
  #text(size: 17pt, weight: "bold", fill: teal, k.value) #text(size: 9pt, fill: muted, k.unit)
]))
#v(10pt)
#text(weight: "semibold", s.monthly_chart)
#image("monthly.svg", width: 100%)
#pagebreak()

= #s.system
#for a in d.arrays [
  #text(weight: "semibold", a.name)
  #grid-table(a.rows)
]

#if d.roof_count > 0 [
  #block(breakable: false)[
    #text(weight: "semibold", s.roof_plan)
    #for i in range(d.roof_count) [#image("roof-" + str(i) + ".svg", width: 100%)]
  ]
]

= #s.site_weather
#grid-table(d.site_rows)
#v(4pt)
#text(size: 8.5pt, fill: muted)[#d.weather_notes.join(" · ") \ #d.licence]

#block(breakable: false)[
  = #s.losses
  #text(size: 9pt, fill: muted, s.losses_intro)
  #image("waterfall.svg", width: 100%)
]
#grid-table(d.loss_rows, widths: (1fr, auto, auto), header: (s.step, s.energy + " (kWh)", s.change), align-cols: (left, right, right))

= #s.monthly_table
#grid-table(d.monthly_rows, widths: (1fr, auto, auto, auto), header: d.monthly_header, align-cols: (left, right, right, right))

= #s.assumptions
#grid-table(d.assumption_rows)
#v(4pt)
#text(size: 9pt)[*#s.models.* #d.models_text]

#if d.notes.len() > 0 [
  = #s.notes
  #for n in d.notes [- #n]
]

#v(10pt)
#block(stroke: (left: 2pt + teal), inset: (left: 8pt, y: 4pt))[#text(size: 9pt, d.disclaimer)]
#v(8pt)
#text(size: 8pt, fill: muted)[*#s.run_info:* #d.run_rows.map(r => r.at(0) + " " + r.at(1)).join(" · ")]
