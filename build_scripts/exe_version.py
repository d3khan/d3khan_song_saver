import json
v = json.load(open('src-tauri/tauri.conf.json', encoding='utf-8')).get('version', '0.0.0')
p = (v.split('.') + ['0', '0', '0'])[:3]
n = [int(''.join(c for c in x if c.isdigit()) or 0) for x in p] + [0]
fv = f"({n[0]},{n[1]},{n[2]},{n[3]})"
print(f"""VSVersionInfo(
  ffi=FixedFileInfo(filevers={fv}, prodvers={fv}, mask=0x3f, flags=0x0,
                    OS=0x40004, fileType=0x1, subtype=0x0, date=(0,0)),
  kids=[StringFileInfo([StringTable('040904B0', [
    StringStruct('CompanyName', 'd3khan'),
    StringStruct('FileDescription', 'd3khan song saver helper'),
    StringStruct('FileVersion', '{v}'),
    StringStruct('InternalName', 'd3khan-helper'),
    StringStruct('LegalCopyright', 'MIT License'),
    StringStruct('OriginalFilename', 'helper.exe'),
    StringStruct('ProductName', 'd3khan song saver'),
    StringStruct('ProductVersion', '{v}')])]),
  VarFileInfo([VarStruct('Translation', [1033, 1200])])]
)""")
