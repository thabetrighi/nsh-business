"""The web app keeps the shop in the browser (IndexedDB): open the page,
try the demo shop, reload, and the shop is still there.

Usage: python3 web_persist.py http://localhost:8000/ /tmp/shots
"""
import sys
from playwright.sync_api import sync_playwright

url, out = sys.argv[1], sys.argv[2]
with sync_playwright() as p:
    ctx = p.chromium.launch_persistent_context(
        "/tmp/nb-web-profile",
        viewport={"width": 1360, "height": 860},
        locale="ar",
    )
    page = ctx.pages[0] if ctx.pages else ctx.new_page()
    page.goto(url)
    page.wait_for_timeout(12000)
    page.screenshot(path=f"{out}/persist-1-first-visit.png")
    # "التجربة بمتجر عرض" on the first screen.
    page.mouse.click(680, 522)
    page.wait_for_timeout(15000)
    page.screenshot(path=f"{out}/persist-2-demo.png")
    page.reload()
    page.wait_for_timeout(15000)
    page.screenshot(path=f"{out}/persist-3-after-reload.png")
    saved = page.evaluate(
        """() => new Promise(done => {
            const r = indexedDB.open('nsh-business');
            r.onsuccess = () => {
                const tx = r.result.transaction('files');
                const g = tx.objectStore('files').get('shop');
                g.onsuccess = () => done(g.result ? g.result.length : 0);
                g.onerror = () => done(-1);
            };
            r.onerror = () => done(-2);
        })"""
    )
    print("bytes kept in IndexedDB:", saved)
    ctx.close()
    if not saved or saved <= 0:
        sys.exit("the shop was not kept in the browser")
