"""Opens the spike twice in the same browser profile to prove persistence."""
import json, sys
from playwright.sync_api import sync_playwright

url, profile, out = sys.argv[1], sys.argv[2], sys.argv[3]
runs = []
with sync_playwright() as p:
    for i in range(2):
        ctx = p.chromium.launch_persistent_context(profile, headless=True)
        page = ctx.new_page()
        page.on("console", lambda m: print("console:", m.text))
        page.goto(url)
        page.wait_for_selector("#out[data-done='1']", timeout=180_000)
        runs.append(json.loads(page.inner_text("#out")))
        ctx.close()
json.dump({"run1_fresh": runs[0], "run2_after_restart": runs[1]}, open(out, "w"), ensure_ascii=False, indent=2)
print(json.dumps(runs, ensure_ascii=False, indent=2))
