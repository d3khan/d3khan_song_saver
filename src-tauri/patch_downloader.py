with open('src/downloader.rs', 'r', encoding='utf-8') as f:
    s = f.read()
s = s.replace('let track = client\n        .resolve(&meta.id)', 'let client_track = client.resolve(&meta.id).map_err(|_| "resolve".into())?;\n    let track = client_track;')
s = s.replace('let mp3_data = encoder.encode_interleaved', 'let _mp3_data = encoder.encode_interleaved')
with open('src/downloader.rs', 'w', encoding='utf-8') as f:
    f.write(s)
print('patch 1 done')
