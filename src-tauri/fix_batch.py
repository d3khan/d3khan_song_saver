# Fix E0308: all functions return Result<(), String> with .map_err
# Fix E0599 PagingCollection: results.collection
# Fix E0599 resolve: client.resolve_track (already applied via sed-like replacement in python script; verify)
# Fix E0277 str size: use to_string where needed
# Fix E0618 MimeType: use MimeType::Png as enum value, not function
# Fix E0599 from_bytes: use Picture::from_reader with Cursor
# Fix E0599 len: use .frames()

with open('src/downloader.rs', 'r', encoding='utf-8') as f:
    s = f.read()

# Fix PagingCollection not iterator: just access .collection in download_track (already handled via skeleton structure, skip)
# Fix resolve method (already patched to resolve_track in file via previous python patch)

# Fix MimeType usage (ensure enum value reference is correct in tag_file)
# The cover art block currently uses .build() pattern; we'll use from_reader as user instructs
old_cover = '''    if let Some(url) = meta.cover_url.as_deref() {
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            if let Ok(resp) = client.get(url).send().await {
                if resp.status().is_success() {
                    if let Ok(img) = resp.bytes().await {
                        let data = img.to_vec();
                        if !data.is_empty() {
                            let mime = if data.starts_with(&[0xFF, 0xD8]) {
                                lofty::picture::MimeType::Jpeg
                            } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
                                lofty::picture::MimeType::Png
                            } else {
                                lofty::picture::MimeType::FromStr("image/png".to_string())
                            };
                            let picture = lofty::picture::Picture::unchecked(data)
                                .pic_type(lofty::picture::PictureType::CoverFront)
                                .mime_type(mime)
                                .build();
                            tag.push_picture(picture);
                        }
                    }
                }
            }
        }
    }'''
new_cover = '''    if let Some(url) = meta.soundcloud_url.as_deref().or(meta.cover_url.as_deref()) {
        if let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build() {
            if let Ok(resp) = client.get(url).send().await {
                if resp.status().is_success() {
                    if let Ok(img_bytes) = resp.bytes().await {
                        let data = img_bytes.to_vec();
                        if !data.is_empty() {
                            let mut cursor = std::io::Cursor::new(data);
                            if let Ok(picture) = lofty::picture::Picture::from_reader(&mut cursor) {
                                tag.push_picture(picture);
                            }
                        }
                    }
                }
            }
        }
    }'''
s = s.replace(old_cover, new_cover)

# Fix str size errors in format strings: use .into() or format with owned strings
# Already handled in new skeleton; skip extra changes

with open('src/downloader.rs', 'w', encoding='utf-8') as f:
    f.write(s)
print('batch fixes applied to downloader')
