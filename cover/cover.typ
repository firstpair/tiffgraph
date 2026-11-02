// tiffgraph cover and headboards, drawn rather than painted:
// a TIFF directory graph above the file it describes, the file as a strip
// coloured by region role, and the one editable XMP packet picked out.
//   typst compile --root . --ppi 140 --input kind=cover cover/cover.typ cover/tiffgraph-cover-drawn.png
//   typst compile --root . --ppi 200 --input kind=headboard cover/cover.typ cover/tiffgraph-headboard-drawn.png
//   typst compile --root . --ppi 200 --input kind=headboard-text cover/cover.typ cover/tiffgraph-headboard-text-drawn.png
#let kind = sys.inputs.at("kind", default: "cover")
#let cover = kind == "cover"
#let pw = if cover { 7.3333in } else { 10.86in }
#let ph = if cover { 11in } else { 3.62in }
#set page(width: pw, height: ph, margin: 0pt, fill: rgb("#0e2230"))
#set text(font: ("Helvetica Neue", "Helvetica", "Arial"), fill: rgb("#e8f0f3"))

#let ink = rgb("#e8f0f3")
#let structure = rgb("#4f7184")
#let standard = rgb("#7fb3c8")
#let private = rgb("#c9a86a")
#let image = rgb("#2f8f83")
#let xmp = rgb("#f2683c")

// The file strip: (fraction of the file, role).
#let strip = (
  (0.02, structure), (0.05, standard), (0.015, structure), (0.04, private),
  (0.03, standard), (0.01, structure), (0.09, xmp), (0.01, structure),
  (0.05, standard), (0.36, image), (0.02, structure), (0.04, private),
  (0.025, standard), (0.17, image), (0.02, structure), (0.04, standard),
)

#let file-strip(width, height) = {
  let total = strip.map(s => s.at(0)).sum()
  stack(dir: ltr, ..strip.map(s => rect(width: width * s.at(0) / total, height: height,
    fill: s.at(1), stroke: 0.6pt + rgb("#0e2230"))))
}

#let node(label, fill: structure, width: 1.25in) = box(
  width: width, inset: (x: 6pt, y: 5pt), radius: 3pt, fill: fill,
  align(center, text(size: 9pt, weight: "bold", fill: rgb("#0e2230"), label)))

#let graph(k) = {
  let s(x) = x * k
  let line-to(a, b) = place(line(start: a, end: b, stroke: 1.2pt + rgb("#9fc3d2")))
  box(width: s(5.4in), height: s(2.4in), {
    line-to((s(0.62in), s(0.33in)), (s(2.7in), s(0.95in)))
    line-to((s(2.7in), s(1.25in)), (s(0.75in), s(2.0in)))
    line-to((s(2.7in), s(1.25in)), (s(2.7in), s(2.0in)))
    line-to((s(2.7in), s(1.25in)), (s(4.65in), s(2.0in)))
    place(dx: s(0in), dy: s(0.1in), scale(k * 100%, origin: top + left, node("header", width: 1.25in)))
    place(dx: s(2.07in), dy: s(0.92in), scale(k * 100%, origin: top + left, node("IFD 0", fill: standard, width: 1.25in)))
    place(dx: s(0.12in), dy: s(1.95in), scale(k * 100%, origin: top + left, node("Exif", fill: standard, width: 1.25in)))
    place(dx: s(2.07in), dy: s(1.95in), scale(k * 100%, origin: top + left, node("XMP", fill: xmp, width: 1.25in)))
    place(dx: s(4.02in), dy: s(1.95in), scale(k * 100%, origin: top + left, node("raw strips", fill: image, width: 1.25in)))
  })
}

#let legend(size) = {
  set text(size: size)
  let item(c, l) = box(baseline: 20%, rect(width: size, height: size, fill: c)) + h(4pt) + l + h(12pt)
  item(structure, "structure") + item(standard, "standard value") + item(private, "private value") + item(image, "image bytes") + item(xmp, "the one editable packet")
}

#if cover {
  place(top + left, dx: 0.7in, dy: 0.9in, block(width: 6in, {
    text(size: 54pt, weight: "bold", "tiffgraph")
    v(-0.25in)
    text(size: 16pt, fill: rgb("#b9d3dd"), "A bounded TIFF/DNG graph walker for OCaml, OxCaml and Rust")
  }))
  place(top + left, dx: 0.85in, dy: 3.4in, graph(1.05))
  place(top + left, dx: 0.7in, dy: 6.55in, file-strip(5.93in, 0.7in))
  place(top + left, dx: 0.7in, dy: 7.4in, block(width: 5.93in, legend(9pt)))
  place(bottom + left, dx: 0.7in, dy: -0.8in, block(width: 6in, {
    text(size: 13pt, "Alexy Khrabrov, with Claude as co-author")
    v(0.05in)
    text(size: 11pt, fill: rgb("#9fbfcc"), "First Pair Press")
  }))
} else {
  let text-on = kind == "headboard-text"
  place(top + left, dx: if text-on { 5.3in } else { 2.7in }, dy: 0.42in, graph(0.78))
  place(top + left, dx: if text-on { 5.3in } else { 0.6in }, dy: 2.55in,
    file-strip(if text-on { 4.95in } else { 9.66in }, 0.42in))
  if text-on {
    place(top + left, dx: 0.6in, dy: 0.95in, block(width: 4.4in, {
      text(size: 44pt, weight: "bold", "tiffgraph")
      v(-0.15in)
      text(size: 14pt, fill: rgb("#b9d3dd"), "Find the XMP packet. Protect everything else.")
    }))
  }
}
