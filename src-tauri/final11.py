with open('src/downloader.rs', 'r', encoding='utf-8') as f:
    s = f.read()

# 5: PagingCollection -> .collection
s = s.replace('results\n        .into_iter()', 'results.collection\n        .into_iter()')
# Ensure collection used correctly
s = s.replace('results.collection\n        .map', 'results.collection.into_iter().map')
# 8: MimeType enum value (not function call). Fix tag_file cover art block
old = 'lofty::picture::MimeType::Png'
# Keep as-is (already correct enum value) but ensure no FromStr call remains
# 9: Picture::from_reader with Cursor (already applied in batch script; verify present)
# If cover block missing Cursor usage, inject it in tag_file area
# 10: .len() -> .frames()
s = s.replace('let num_frames = buf.len();', 'let num_frames = buf.frames();')
# 11: tag.push_picture(picture) (already in new block; ensure variable name is `picture` not `cover`)
# 4: all functions return Result with to_string (already applied resolve fix with .to_string())
# 7: to_string on slices: check .map_err uses format! which is fine

with open('src/downloader.rs', 'w', encoding='utf-8') as f:
    f.write(s)
print('final batch applied')
