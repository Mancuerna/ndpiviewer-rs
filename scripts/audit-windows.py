"""Inspect a Windows distribution PE without loading it (Python standard library only)."""
import json
import pathlib
import struct
import sys

path = pathlib.Path(sys.argv[1])
data = path.read_bytes()
u16 = lambda offset: struct.unpack_from('<H', data, offset)[0]
u32 = lambda offset: struct.unpack_from('<I', data, offset)[0]
pe = u32(0x3c)
assert data[pe:pe+4] == b'PE\0\0'
assert u16(pe + 4) == 0x8664, 'Expected x64 executable'
optional = pe + 24
assert u16(optional) == 0x20b
assert u16(optional + 68) == 2, 'Expected Windows GUI subsystem'
section_table = optional + u16(pe + 20)
sections = []
for index in range(u16(pe + 6)):
    entry = section_table + index * 40
    sections.append((u32(entry + 12), max(u32(entry + 8), u32(entry + 16)), u32(entry + 20)))

def offset(rva):
    for start, size, raw in sections:
        if start <= rva < start + size:
            return raw + rva - start
    raise ValueError('Invalid RVA')

def name(rva):
    pos = offset(rva)
    return data[pos:data.index(b'\0', pos)].decode('ascii').lower()

imports = set()
for directory, stride, name_offset in [(1, 20, 12), (13, 32, 4)]:
    rva = u32(optional + 112 + directory * 8)
    if not rva:
        continue
    pos = offset(rva)
    while any(data[pos:pos+stride]):
        imports.add(name(u32(pos + name_offset)))
        pos += stride
system = {
    'kernel32.dll', 'user32.dll', 'gdi32.dll', 'advapi32.dll', 'shell32.dll',
    'ole32.dll', 'oleaut32.dll', 'comdlg32.dll', 'comctl32.dll', 'shlwapi.dll',
    'dwmapi.dll', 'opengl32.dll', 'imm32.dll', 'version.dll', 'ws2_32.dll',
    'bcrypt.dll', 'bcryptprimitives.dll', 'ntdll.dll', 'crypt32.dll', 'secur32.dll', 'userenv.dll',
    'propsys.dll', 'uxtheme.dll', 'dwrite.dll', 'msimg32.dll', 'winmm.dll',
    'rpcrt4.dll', 'winspool.drv', 'normaliz.dll', 'windowscodecs.dll',
    'setupapi.dll', 'cfgmgr32.dll', 'shcore.dll', 'usp10.dll', 'msvcrt.dll',
}
unexpected = [dll for dll in imports if dll not in system and not dll.startswith(('api-ms-win-', 'ext-ms-win-'))]
assert not unexpected, f'Non-system runtime dependencies: {unexpected}'

# Windows Explorer uses the executable's icon resources, independently of egui.
resource_rva = u32(optional + 112 + 2 * 8)
assert resource_rva, 'Application resources are missing'
resource_root = offset(resource_rva)
resource_count = u16(resource_root + 12) + u16(resource_root + 14)
resource_types = {u32(resource_root + 16 + index * 8) for index in range(resource_count)}
assert {3, 14}.issubset(resource_types), 'Application icon and icon group must be embedded'
debug_rva, debug_size = struct.unpack_from('<II', data, optional + 112 + 6 * 8)
if debug_rva:
    for pos in range(offset(debug_rva), offset(debug_rva) + debug_size, 28):
        assert u32(pos + 12) != 2, 'CodeView/PDB debug data must not ship'
for text in ('example_images', 'JNOC-GC2003', 'Synthetic_fluorescence', 'C:\\Users\\', 'C:/Users/', '--cache-benchmark'):
    for encoding in ('utf-8', 'utf-16le'):
        assert text.lower().encode(encoding) not in data.lower(), f'Development data: {text}'
print(json.dumps({'file': path.name, 'bytes': len(data), 'architecture': 'x64', 'subsystem': 'GUI', 'imports': sorted(imports), 'icon_embedded': True, 'codeview': False, 'private_path_scan': 'passed'}, indent=2))
