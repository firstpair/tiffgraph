#!/usr/bin/env python3
"""The OCaml and Rust builds of tiffgraph must agree on every file: the same
packet or the same refusal reason under each grammar, the same unknown-field
survey, and the same component regions, line for line.

Files: synthetic DNG-shaped TIFFs built here (both byte orders, classic and
BigTIFF, sub-IFDs, Exif, camera profiles, private fields, strips and tiles),
and a few thousand seeded mutations of them (byte flips, truncations, counts
and offsets rewritten). Extra paths on the command line, such as real DNGs,
are compared too; nothing is ever written next to them."""
import argparse, pathlib, random, struct, subprocess, sys, tempfile

p = argparse.ArgumentParser()
p.add_argument('--ocaml', required=True, help='test/dump.exe from the OCaml build')
p.add_argument('--rust', required=True, help='the Rust build\'s dump example')
p.add_argument('--mutations', type=int, default=3000)
p.add_argument('paths', nargs='*', help='more files to compare')
a = p.parse_args()

def build(little=True, big=False, entries=(), sub=None, exif=None, profiles=(), strip=b'\xaa' * 64, trailer=b''):
    """A TIFF from (tag, type, count, bytes) entries, values placed after each IFD."""
    e = '<' if little else '>'
    cw, nw, inline = (8, 8, 8) if big else (2, 4, 4)
    out = bytearray(b'II' if little else b'MM')
    out += struct.pack(e + 'H', 43 if big else 42)
    if big:
        out += struct.pack(e + 'HH', 8, 0)
    first = len(out); out += b'\0' * nw
    patches = []
    def num(v, w):
        return v.to_bytes(w, 'little' if little else 'big')
    def ifd(items):
        at = len(out)
        out.extend(num(len(items), cw))
        slots = []
        for tag, typ, count, data in items:
            out.extend(num(tag, 2) + num(typ, 2) + num(count, nw))
            slots.append(len(out)); out.extend(b'\0' * inline)
        out.extend(b'\0' * nw)
        for (tag, typ, count, data), slot in zip(items, slots):
            if callable(data):
                patches.append((slot, data))
            elif len(data) <= inline:
                out[slot:slot + len(data)] = data
            else:
                out[slot:slot + nw] = num(len(out), nw); out.extend(data)
        return at
    def entry_long(v):
        return num(v, 4)
    items = list(entries)
    strip_slot = []
    items.append((273, 4, 1, lambda pos: num(pos['strip'], 4)))
    items.append((279, 4, 1, num(len(strip), 4)))
    items.append((50706, 1, 4, b'\x01\x04\x00\x00'))
    if sub is not None:
        items.append((330, 4, 1, lambda pos: num(pos['sub'], 4)))
    if exif is not None:
        items.append((34665, 4, 1, lambda pos: num(pos['exif'], 4)))
    if profiles:
        items.append((50933, 4, len(profiles), lambda pos: b''.join(num(x, 4) for x in pos['profiles'])[:4] if len(profiles) == 1 else None))
    pos = {}
    ifd0 = ifd(items)
    if sub is not None:
        pos['sub'] = ifd(sub)
    if exif is not None:
        pos['exif'] = ifd(exif)
    if profiles:
        pos['profiles'] = []
        for prof in profiles:
            base = len(out)
            out.extend((b'II' if little else b'MM') + num(0x4352, 2) + num(8, 4))
            sav = len(out)
            # a classic IFD relative to base
            out.extend(num(len(prof), 2))
            pslots = []
            for tag, typ, count, data in prof:
                out.extend(num(tag, 2) + num(typ, 2) + num(count, 4)); pslots.append(len(out)); out.extend(b'\0' * 4)
            out.extend(b'\0' * 4)
            for (tag, typ, count, data), slot in zip(prof, pslots):
                if len(data) <= 4:
                    out[slot:slot + len(data)] = data
                else:
                    out[slot:slot + 4] = num(len(out) - base, 4); out.extend(data)
            pos['profiles'].append(base)
    pos['strip'] = len(out); out.extend(strip); out.extend(trailer)
    out[first:first + nw] = num(ifd0, nw)
    for slot, f in patches:
        d = f(pos)
        if d is not None:
            out[slot:slot + len(d)] = d
    return bytes(out)

def xmp(n, typ=1):
    return (700, typ, n, (b'<x:xmpmeta/>' * (n // 12 + 1))[:n])

def short(tag, v, little=True):
    return (tag, 3, 1, v.to_bytes(2, 'little' if little else 'big'))

def long_(tag, v, little=True):
    return (tag, 4, 1, v.to_bytes(4, 'little' if little else 'big'))

rng = random.Random(20261102)
seeds = []
for little in (True, False):
    for big in (False, True):
        base = [short(256, 8, little), short(257, 4, little), xmp(120)]
        seeds.append(build(little, big, base))
        seeds.append(build(little, big, base + [long_(65001, 7, little), (65003, 7, 8, b'abcdefgh')]))
        seeds.append(build(little, big, base + [short(65002, 9, little)]))
        seeds.append(build(little, big, base, exif=[(37500, 7, 15, b'MakerNote-bytes'), (33434, 5, 1, b'\0\0\0\1\0\0\0\2')]))
        seeds.append(build(little, big, base, sub=[long_(254, 1, little), xmp(40)]))
        seeds.append(build(little, big, base + [(50740, 1, 8, b'private!'), long_(37393, 3, little)]))
        seeds.append(build(little, big, [short(256, 8, little), xmp(120, 7)]))
        seeds.append(build(little, big, base + [(52547, 3, 1, (2).to_bytes(2, 'little' if little else 'big'))]))
        seeds.append(build(little, big, base, profiles=[[(50708, 2, 6, b'Adobe\0'), (50936, 2, 5, b'name\0')]]))
        seeds.append(build(little, big, [short(256, 8, little)]))

with tempfile.TemporaryDirectory(prefix='tiffgraph-cross-') as td:
    t = pathlib.Path(td)
    files = []
    for i, s in enumerate(seeds):
        f = t / ('seed-%03d.tif' % i); f.write_bytes(s); files.append(f)
    for i in range(a.mutations):
        s = bytearray(rng.choice(seeds))
        kind = rng.randrange(5)
        if kind == 0:
            for _ in range(rng.randint(1, 4)):
                s[rng.randrange(len(s))] = rng.randrange(256)
        elif kind == 1:
            s = s[:rng.randrange(len(s))]
        elif kind == 2:
            j = rng.randrange(max(1, min(len(s), 200)))
            s[j:j + 4] = rng.choice([b'\xff\xff\xff\xff', b'\0\0\0\0', b'\x08\0\0\0', b'\x7f\xff\xff\xff'])
        elif kind == 3:
            j = rng.randrange(max(1, min(len(s), 200)))
            s[j:j + 8] = bytes(rng.randrange(256) for _ in range(8))
        else:
            j = rng.randrange(len(s)); s = s[:j] + s[j:j + rng.randint(1, 64)] * 2 + s[j:]
        f = t / ('mut-%05d.tif' % i); f.write_bytes(bytes(s)); files.append(f)
    files += [pathlib.Path(x) for x in a.paths]
    bad = 0; checked = 0
    for k in range(0, len(files), 200):
        chunk = [str(f) for f in files[k:k + 200]]
        o = subprocess.run([a.ocaml, *chunk], capture_output=True, text=True, errors='replace')
        r = subprocess.run([a.rust, *chunk], capture_output=True, text=True, errors='replace')
        if o.returncode or r.returncode:
            sys.exit('FAIL: a dump crashed:\n' + o.stderr[-2000:] + r.stderr[-2000:])
        if o.stdout != r.stdout:
            ol, rl = o.stdout.splitlines(), r.stdout.splitlines()
            for x, y in zip(ol, rl):
                if x != y:
                    print('first difference:\n  ocaml: %s\n  rust:  %s' % (x, y)); break
            else:
                print('outputs differ in length: %d vs %d lines' % (len(ol), len(rl)))
            bad += 1
            break
        checked += len(chunk)
    outcomes = {}
    if bad:
        sys.exit('FAIL: the builds disagree')
    print('ok: OCaml and Rust agree on %d files (%d synthetic seeds, %d mutations, %d given)'
          % (checked, len(seeds), a.mutations, len(a.paths)))
