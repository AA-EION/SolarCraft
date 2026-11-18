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
#show heading.where(level: 1): it => block(sticky: true, above: 1.4em, below: 0.7em)[#text(size: 14pt, weight: "semibold", fill: teal, it.body)]

#let grid-table(rows, widths: (1fr, auto), header: none, align-cols: (left, right)) = {
  let cells = ()
  if header != none {
    cells.push(table.header(..header.map(h => text(weight: "semibold", size: 9pt, fill: muted, h))))
  }
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
#for sec in d.contents [
  #if sec.id == "equipment" and d.report_type != "customer_short" { pagebreak(weak: true) }
  = #sec.title
  #for (i, b) in sec.blocks.enumerate() [
    #if b.kind == "text" [
      #let next = sec.blocks.at(i + 1, default: none)
      #block(sticky: next != none and next.kind != "text")[#text(size: 9.5pt)[#b.text]]
    ] else if b.kind == "table" [
      #let n = b.rows.first().len()
      #let widths = if n == 2 { (0.44fr, 0.56fr) } else { (1.8fr,) + range(n - 1).map(_ => 1fr) }
      #block(breakable: not ("assumptions", "run").contains(sec.id))[
        #grid-table(b.rows, widths: widths, header: if b.header.len() > 0 { b.header } else { none },
          align-cols: (left,) + range(n - 1).map(_ => right))
      ]
    ] else if b.kind == "figure" [
      #block(breakable: false)[
        #if b.caption != "" [#text(weight: "semibold")[#b.caption] #v(3pt)]
        #image(b.file, width: 100%)
      ]
    ]
  ]
]
