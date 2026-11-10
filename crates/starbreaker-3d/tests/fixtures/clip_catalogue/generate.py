"""Generate tiny, entirely invented DBA/CAF/CryXmlB fixtures; no game input."""
from pathlib import Path
import struct
p = Path(__file__).parent
pack = struct.pack
# Two block headers followed by shared key pools; one invented bone hash each.
blocks = bytearray(pack('<I', 0))
for i in range(2):
    blocks += b'#dba' + pack('<HHI', 1, 0xAA55, 40) + pack('<I', 0x12345678)
    blocks += bytes(24)
for i in range(2):
    ctrl = 4 + i * 40 + 16
    rot_offset = len(blocks) - ctrl
    for _ in range(3): blocks += pack('<4f', 0, 0, 0, 1)
    pos_offset = len(blocks) - ctrl
    for x in ([0, 5, 0] if i == 0 else [0, 4, 8]): blocks += pack('<3f', x, 0, 0)
    blocks[ctrl:ctrl+24] = pack('<HHIIHHII', 3, 0x8000, 0, rot_offset, 3, 0xC000, 0, pos_offset)
blocks[:4] = pack('<I', len(blocks))
meta = bytearray(pack('<I', 2))
for _ in range(2):
    meta += pack('<IIHHIII6f', 0, 0, 24, 1, 0x900, 0, 2, 0, 0, 0, 1, 0, 0)
meta += bytes((-len(meta)) % 8)
meta += b'animations/synthetic/cycle.caf\0animations/synthetic/spare.caf\0'
header = b'#ivo' + pack('<III', 0x900, 2, 16)
table = pack('<IIQIIQ', 0x194FBC50, 0x900, 48, 0xF7351608, 0x902, 48 + len(blocks))
(p/'synthetic.dba').write_bytes(header + table + blocks + meta)
# Single CAF uses the same two-block payload for exercising both endpoint summaries.
info = pack('<IHH', 0, 24, 0)
table = pack('<IIQIIQ', 0xA9496CB5, 0x900, 48, 0x4733C6ED, 0x900, 48 + len(blocks))
(p/'synthetic.caf').write_bytes(header + table + blocks + info)
def chrparams(rows):
    """CryXmlB `<Params>` with one `<Animation name= path=>` child per row."""
    strings = bytearray(b'\0')
    def string(s):
        offset = len(strings)
        strings.extend(s.encode() + b'\0')
        return offset
    nodes, attrs = [], []
    children = list(range(1, len(rows) + 1))
    nodes.append(pack('<IIHHiiii', string('Params'), 0, 0, len(rows), -1, 0, 0, 0))
    for name, path in rows:
        nodes.append(pack('<IIHHiiii', string('Animation'), 0, 2, 0, 0, len(attrs), 0, 0))
        attrs += [pack('<II', string('name'), string(name)), pack('<II', string('path'), string(path))]
    node_pos = 44
    child_pos = node_pos + len(nodes)*28
    attr_pos = child_pos + len(children)*4
    str_pos = attr_pos + len(attrs)*8
    xml = b'CryXmlB\0' + pack('<9I', str_pos + len(strings), node_pos, len(nodes), attr_pos, len(attrs), child_pos, len(children), str_pos, len(strings))
    return xml + b''.join(nodes) + pack(f'<{len(children)}i', *children) + b''.join(attrs) + strings
xml = chrparams([('#filepath', 'animations/synthetic'), ('$TracksDatabase', 'animations/synthetic/test.dba'), ('latch_cycle', 'cycle.caf')])
(p/'synthetic.chrparams').write_bytes(xml)
# A loose CAF (no tracks database): one block moving the invented bone `synthetic_latch` 0 -> 4 -> 8.
import zlib
loose = bytearray(pack('<I', 0))
loose += b'#caf' + pack('<HHI', 1, 0xAA55, 40) + pack('<I', zlib.crc32(b'synthetic_latch')) + bytes(24)
rot_offset = len(loose) - 20
for _ in range(3): loose += pack('<4f', 0, 0, 0, 1)
pos_offset = len(loose) - 20
for x in [0, 4, 8]: loose += pack('<3f', x, 0, 0)
loose[20:44] = pack('<HHIIHHII', 3, 0x8000, 0, rot_offset, 3, 0xC000, 0, pos_offset)
loose[:4] = pack('<I', len(loose))
loose_caf = header + pack('<IIQIIQ', 0xA9496CB5, 0x900, 48, 0x4733C6ED, 0x900, 48 + len(loose)) + bytes(loose) + info
empty_ivo = b'#ivo' + pack('<III', 0x900, 0, 16)
# Small deterministic stored ZIP readable by MappedP4k, with one broken rig.
# Everything inside is generated above; this is not a game archive.
import zipfile
with zipfile.ZipFile(p/'rigs.zip', 'w', compression=zipfile.ZIP_STORED) as archive:
    for name, payload in [
        ('Data/Objects/synthetic/good.chr', empty_ivo),
        ('Data/Objects/synthetic/good.chrparams', xml),
        ('Data/Objects/synthetic/broken.chrparams', xml.replace(b'test.dba', b'lost.dba')),
        ('Data/Animations/synthetic/test.dba', (p/'synthetic.dba').read_bytes()),
        # Rig whose events name loose CAFs: one stored, one stored but not a CAF, one stored nowhere.
        ('Data/Objects/synthetic/loose.chrparams', chrparams([('#filepath', 'animations/synthetic'), ('latch_cycle', 'loose.caf'), ('latch_torn', 'torn.caf'), ('latch_absent', 'absent.caf')])),
        # Rig whose first event (in name order) is a loose CAF and whose second is a database entry.
        ('Data/Objects/synthetic/mixed.chrparams', chrparams([('#filepath', 'animations/synthetic'), ('$TracksDatabase', 'animations/synthetic/test.dba'), ('a_loose', 'loose.caf'), ('latch_cycle', 'cycle.caf')])),
        ('Data/Animations/synthetic/loose.caf', loose_caf),
        ('Data/Animations/synthetic/torn.caf', empty_ivo),
    ]:
        archive.writestr(zipfile.ZipInfo(name, (2000, 1, 1, 0, 0, 0)), payload)
