from apify_client import ApifyClient

client = ApifyClient("YOUR_APIFY_TOKEN")
run = client.actor("omkar-cloud/spotify-scraper").call(run_input={
    "playlistUrls": ["https://open.spotify.com/playlist/7JbGYbGThZDPLfLK2nPX1c"]
})

dataset_id = run["defaultDatasetId"]
items = list(client.dataset(dataset_id).iterate_items())
print(f"Fetched {len(items)} tracks")
for t in items[:3]:
    print(f"  • {t.get('name')} — {', '.join(a.get('name','') for a in t.get('artists',[]))}")