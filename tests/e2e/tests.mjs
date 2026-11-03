// Feature test cases for runner.mjs. Selectors use the en-US UI strings of the
// upstream app (web/packages/base/locales/en-US/translation.json) and ARIA
// labels, never internal class names.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const USER1 = { email: "e2e@example.com", password: "Photovault-e2e-Pass-1!" };
const USER2 = { email: "e2e2@example.com", password: "Photovault-e2e-Pass-2!" };
const NEW_PASSWORD = "Photovault-e2e-Pass-1-changed!";
const ALBUM = "E2E Uploads";
const ALBUM2 = "E2E Album 2";

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
const btn = (page, name) => page.getByRole("button", { name, exact: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const expectVisible = async (locator, what, timeout = 30000) => {
    try {
        await locator.first().waitFor({ state: "visible", timeout });
    } catch {
        throw new Error(`expected to see ${what}`);
    }
};

/** True if `locator` becomes visible within `ms` (isVisible() does not wait). */
const appears = (locator, ms = 3000) =>
    locator
        .first()
        .waitFor({ state: "visible", timeout: ms })
        .then(() => true)
        .catch(() => false);

// The album header's share button is labelled by sharing state.
const shareButton = (page) =>
    page.getByRole("button", { name: /^(Share album|Modify sharing|Sharing details)$/ }).first();

const escape = async (page, n = 2) => {
    for (let i = 0; i < n; i++) {
        await page.keyboard.press("Escape");
        await sleep(200);
    }
};

/** Sign up with email + OTT; returns the recovery key mnemonic shown. */
const signup = async (ctx, page, user) => {
    const { config } = ctx;
    await page.goto(`${config.web}/signup`);
    await page.locator("input[name=email]").fill(user.email);
    await page.locator("input[name=password]").fill(user.password);
    await page.locator("input[name=confirmPassword]").fill(user.password);
    // The landing page animates, so the checkbox is never "stable": click it in the DOM.
    const terms = page.locator("input[name=acceptedTerms]");
    for (let i = 0; i < 5 && !(await terms.isChecked()); i++) {
        await terms.evaluate((el) => el.click());
        await sleep(300);
    }
    if (!(await terms.isChecked())) throw new Error("could not tick the terms checkbox");
    await btn(page, "Create account").click();
    await page.waitForURL("**/verify", { timeout: 120000 });
    await page.getByLabel("Verification code 1").waitFor();
    await page.getByLabel("Verification code 1").click();
    await page.keyboard.type(config.ott);
    // The form submits itself after six digits; press Verify if it is still there.
    await sleep(1000);
    if (page.url().endsWith("/verify") && (await btn(page, "Verify").isVisible()))
        await btn(page, "Verify").click();
    await page.waitForURL("**/generate", { timeout: 120000 });
    await expectVisible(page.getByText("Your recovery key"), "the recovery key screen");
    const text = await page.locator("body").innerText();
    const mnemonic = text.match(/\b([a-z]+ ){23}[a-z]+\b/)?.[0];
    if (!mnemonic) throw new Error("no 24-word recovery key on the recovery key screen");
    const [download] = await Promise.all([page.waitForEvent("download"), btn(page, "Save Key").click()]);
    const saved = readFileSync(await download.path(), "utf8").trim();
    if (saved !== mnemonic) throw new Error("saved recovery key file differs from the key shown");
    await page.waitForURL(/\/(gallery|plan)/, { timeout: 120000 });
    if (page.url().includes("/plan")) await page.getByText("Continue with free plan").click();
    await page.waitForURL("**/gallery", { timeout: 120000 });
    return mnemonic;
};

/** Log in with email + password (SRP), ending on /gallery. */
const login = async (ctx, page, user, password = user.password) => {
    await page.goto(`${ctx.config.web}/`);
    await page.locator("input[name=email]").fill(user.email);
    await btn(page, "Login").first().click();
    await page.waitForURL("**/credentials", { timeout: 60000 });
    const pw = page.locator("input[name=password]");
    await pw.waitFor();
    await pw.fill(password);
    await btn(page, "Sign in").click();
    await page.waitForURL("**/gallery", { timeout: 120000 });
};

const openSettings = async (page) => {
    if (await btn(page, "Logout").isVisible().catch(() => false)) return;
    await escape(page);
    await dismissToasts(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await expectVisible(page.getByText("Logout", { exact: true }), "the settings drawer");
};

const logout = async (page) => {
    await openSettings(page);
    await page.getByText("Logout", { exact: true }).click();
    // Confirmation dialog.
    const confirm = page.getByRole("dialog").getByRole("button", { name: "Logout" });
    if (await confirm.isVisible().catch(() => false)) await confirm.click();
    await page.waitForURL((u) => !u.pathname.startsWith("/gallery"), { timeout: 30000 });
};

/** Close "Download complete" and similar toasts, which cover the sidebar. */
const dismissToasts = async (page) => {
    for (const text of ["Download complete", "Upload complete"]) {
        const toast = page.getByText(text, { exact: true });
        if (!(await toast.first().isVisible().catch(() => false))) continue;
        const close = toast.first().locator("xpath=ancestor::*[.//button][1]").getByRole("button").last();
        await close.click().catch(() => {});
    }
};

const openSidebarAlbum = async (page, name) => {
    await escape(page);
    await dismissToasts(page);
    const clear = page.getByRole("button", { name: "Clear search" });
    if (await clear.first().isVisible().catch(() => false)) await clear.first().click();
    const target = page.getByRole("button", { name, exact: true }).first();
    if (!(await target.isVisible().catch(() => false))) {
        // e.g. still inside the "Hidden albums" section: go back to all albums.
        const back = page.getByRole("button", { name: "Back" });
        if (await back.first().isVisible().catch(() => false)) await back.first().click();
        if (!(await target.isVisible().catch(() => false)))
            await page.getByRole("button", { name: "Albums", exact: true }).first().click();
    }
    await target.click();
    await sleep(1000);
};

// Gallery items (the sidebar's album covers are blob: images too).
const thumbnails = (page) => page.locator(".photo-list img[src^='blob:']");

// Each item has an overlay that intercepts pointer events, so click "through" it.
const openItem = async (page, i = 0) => {
    await thumbnails(page).nth(i).click({ force: true });
    await expectVisible(page.locator(".pswp"), "the photo viewer");
};

/** If the app asks for the password again (e.g. for Hidden), give it. */
const reauthIfAsked = async (page, password) => {
    const pw = page.getByRole("dialog").locator("input[type=password]");
    if (await appears(pw, 5000)) {
        await pw.first().fill(password);
        const auth = page.getByRole("dialog").getByRole("button", { name: "Authenticate" });
        if (await auth.isVisible().catch(() => false)) await auth.click();
        else await page.keyboard.press("Enter");
        await sleep(2000);
    }
};

const waitThumbnails = async (page, n, timeout = 60000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
        if ((await thumbnails(page).count()) >= n) return;
        await sleep(500);
    }
    throw new Error(`expected at least ${n} thumbnails, saw ${await thumbnails(page).count()}`);
};

/** Select the gallery item whose thumbnail is the i-th one (hover shows its checkbox). */
const selectItem = async (page, i = 0) => {
    const thumb = thumbnails(page).nth(i);
    await thumb.hover({ force: true });
    const box = thumb
        .locator("xpath=ancestor::*[.//input[@type='checkbox']][1]")
        .locator("input[type=checkbox]")
        .first();
    await box.evaluate((el) => el.click());
    await expectVisible(page.getByText(/^\d+ selected$/), "the selection toolbar", 10000);
};

const uploadFiles = async (page, files, albumName) => {
    await btn(page, "Upload").first().click();
    const [chooser] = await Promise.all([page.waitForEvent("filechooser"), btn(page, "Files").click()]);
    await chooser.setFiles(files);
    // "Upload to album" picker.
    await page.getByText("New album", { exact: true }).last().click();
    await page.locator("input[name=value]").fill(albumName);
    await btn(page, "Create").click();
    await btn(page, "Start upload").click();
    await expectVisible(page.getByText(`${files.length} uploaded`), "the upload to complete", 120000);
};

/** Click a button in the selection toolbar by its ARIA label. */
const toolbar = (page, name) => page.getByRole("button", { name, exact: true }).first();

const waitNoThumbnails = async (page, what, timeout = 30000) => {
    const deadline = Date.now() + timeout;
    while ((await thumbnails(page).count()) > 0) {
        if (Date.now() > deadline) throw new Error(what);
        await sleep(500);
    }
};

const confirmDialog = async (page, name) => {
    const b = page.getByRole("dialog").getByRole("button", { name });
    await b.first().click();
};

export const tests = [
    {
        name: "web_app_loads",
        async run(ctx) {
            // The build points at our API origin (NEXT_PUBLIC_ENTE_ENDPOINT); the
            // landing page shows the custom host.
            const page = await ctx.newPage("smoke");
            await page.goto(`${ctx.config.web}/`);
            await expectVisible(page.getByText(new URL(ctx.config.api).host), "the custom API host on the landing page");
            await expectVisible(page.locator("input[name=email]"), "the login form");
        },
    },
    {
        name: "dev_settings_custom_endpoint",
        async run(ctx) {
            // The official way to point any build at a self-hosted server: seven
            // taps on the landing page background open "Developer settings",
            // which validates the origin with GET /ping ({"message":"pong"}).
            const page = await ctx.newPage("devsettings");
            await page.goto(`${ctx.config.web}/`);
            const background = page.getByText("Safe home for your photos").first();
            await background.waitFor();
            for (let i = 0; i < 7; i++) {
                await background.click();
                await sleep(150);
            }
            await expectVisible(page.getByText("Developer settings"), "the developer settings dialog");
            await page.locator("input[name=apiOrigin]").fill(ctx.config.api);
            await btn(page, "Save").click();
            const gone = await page
                .getByText("Developer settings")
                .waitFor({ state: "hidden", timeout: 15000 })
                .then(() => true)
                .catch(() => false);
            if (!gone) throw new Error('developer settings rejected the endpoint (GET /ping must return {"message":"pong"})');
            await expectVisible(page.getByText(new URL(ctx.config.api).host), "the custom API host on the landing page");
        },
    },
    {
        name: "signup",
        async run(ctx) {
            const page = await ctx.newPage("user1");
            ctx.state.user1 = { page, recoveryKey: await signup(ctx, page, USER1) };
            await expectVisible(page.getByText("Upload your first photo"), "the empty gallery");
        },
    },
    {
        name: "logout_login",
        deps: ["signup"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await logout(page);
            await login(ctx, page, USER1);
        },
    },
    {
        name: "upload_photos_and_video",
        deps: ["signup"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            const { files } = ctx.media;
            await uploadFiles(page, [files.photo1, files.photo2, files.video1], ALBUM);
            await escape(page);
            await expectVisible(page.getByRole("button", { name: ALBUM, exact: true }), `album "${ALBUM}" in the sidebar`);
            await waitThumbnails(page, 3);
        },
    },
    {
        name: "gallery_after_reload",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            // A fresh load must fetch collections and the file diff from the server.
            const { page } = ctx.state.user1;
            await page.reload();
            await page.waitForURL("**/gallery");
            await expectVisible(page.getByText("3 memories"), '"3 memories"', 60000);
            await waitThumbnails(page, 3);
        },
    },
    {
        name: "open_photo",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await escape(page);
            // Newest first: the last item is the first photo uploaded.
            await openItem(page, (await thumbnails(page).count()) - 1);
            // The full-size file is fetched from object storage and decrypted.
            const deadline = Date.now() + 60000;
            for (;;) {
                const ok = await page
                    .locator(".pswp img.pswp__img")
                    .evaluateAll((imgs) => imgs.some((i) => i.complete && i.naturalWidth >= 640))
                    .catch(() => false);
                if (ok) break;
                if (Date.now() > deadline) throw new Error("full-size image did not load in the viewer");
                await sleep(500);
            }
            await escape(page, 1);
        },
    },
    {
        name: "play_video",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await escape(page);
            await openItem(page, 0); // the video is the newest item
            const deadline = Date.now() + 60000;
            for (;;) {
                const ok = await page
                    .locator(".pswp video")
                    .evaluateAll((vs) => vs.some((v) => v.readyState >= 1 && v.videoWidth === 320))
                    .catch(() => false);
                if (ok) break;
                if (Date.now() > deadline) throw new Error("video did not load in the viewer");
                await sleep(500);
            }
            await escape(page, 1);
        },
    },
    {
        name: "download_matches_original",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            const want = new Set(Object.values(ctx.media.sha));
            await escape(page);
            await openItem(page, (await thumbnails(page).count()) - 1);
            await page.locator(".pswp").getByRole("button", { name: "More" }).click();
            const [download] = await Promise.all([
                page.waitForEvent("download", { timeout: 60000 }),
                page.getByRole("menuitem", { name: "Download" }).click(),
            ]);
            const got = sha256(readFileSync(await download.path()));
            if (!want.has(got)) throw new Error(`downloaded ${download.suggestedFilename()} does not match any uploaded original`);
            ctx.note(`downloaded ${download.suggestedFilename()} matches the original byte for byte`);
            await escape(page, 2);
            await dismissToasts(page);
        },
    },
    {
        name: "create_album",
        deps: ["signup"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await escape(page);
            await page.getByRole("button", { name: "New album" }).first().click();
            await page.locator("input[name=value]").fill(ALBUM2);
            await btn(page, "Create").click();
            await sleep(1500);
            await escape(page);
            await expectVisible(page.getByRole("button", { name: ALBUM2, exact: true }), `album "${ALBUM2}" in the sidebar`);
        },
    },
    {
        name: "add_and_remove_from_album",
        deps: ["upload_photos_and_video", "create_album"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSidebarAlbum(page, ALBUM);
            await waitThumbnails(page, 3);
            await selectItem(page, 0);
            await toolbar(page, "Add").click();
            await page.getByRole("dialog").getByText(ALBUM2, { exact: true }).first().click();
            await sleep(2000);
            await escape(page);
            await openSidebarAlbum(page, ALBUM2);
            await waitThumbnails(page, 1);
            await selectItem(page, 0);
            await page.getByRole("button", { name: /remove/i }).first().click();
            const confirm = page.getByRole("dialog").getByRole("button", { name: /remove/i });
            if (await appears(confirm, 3000)) await confirm.first().click();
            await waitNoThumbnails(page, "file still shown in the album after removing it");
        },
    },
    {
        name: "favorite",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSidebarAlbum(page, ALBUM);
            await waitThumbnails(page, 1);
            await openItem(page, (await thumbnails(page).count()) - 1);
            await page.locator(".pswp").getByRole("button", { name: "Favorite" }).click();
            await sleep(2000);
            await escape(page);
            await expectVisible(page.getByRole("button", { name: "Favorites", exact: true }), 'a "Favorites" album in the sidebar');
            await openSidebarAlbum(page, "Favorites");
            await waitThumbnails(page, 1);
        },
    },
    {
        name: "hide_and_unhide",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSidebarAlbum(page, ALBUM);
            await waitThumbnails(page, 2);
            const before = await thumbnails(page).count();
            await selectItem(page, before - 1);
            await toolbar(page, "Hide").click();
            const confirm = page.getByRole("dialog").getByRole("button", { name: /hide/i });
            if (await appears(confirm, 3000)) await confirm.first().click();
            await sleep(2000);
            await openSidebarAlbum(page, "Hidden");
            await reauthIfAsked(page, USER1.password);
            await waitThumbnails(page, 1);
            await selectItem(page, 0);
            await toolbar(page, "Unhide").click();
            // "Unhide to album" picker.
            await page.getByRole("dialog").getByText(ALBUM, { exact: true }).first().click();
            await waitNoThumbnails(page, "file still in Hidden after unhiding it");
            await openSidebarAlbum(page, ALBUM);
            await waitThumbnails(page, before);
        },
    },
    {
        name: "trash_restore_delete",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSidebarAlbum(page, ALBUM);
            await waitThumbnails(page, 1);
            const before = await thumbnails(page).count();
            const trashOne = async () => {
                await selectItem(page, (await thumbnails(page).count()) - 1);
                await toolbar(page, "Delete").click();
                await confirmDialog(page, /move to trash|delete/i);
                await sleep(2000);
            };
            await trashOne();
            await openSidebarAlbum(page, "Trash");
            await waitThumbnails(page, 1);
            await selectItem(page, 0);
            await toolbar(page, "Restore").click();
            const pick = page.getByRole("dialog").getByText(ALBUM, { exact: true });
            if (await appears(pick, 3000)) await pick.first().click();
            await sleep(2000);
            await openSidebarAlbum(page, ALBUM);
            await waitThumbnails(page, before);
            await trashOne();
            await openSidebarAlbum(page, "Trash");
            await waitThumbnails(page, 1);
            // Icon button whose tooltip wrapper carries the label.
            await page.locator("[aria-label='Empty trash'] button, button[aria-label='Empty trash']").first().click();
            await confirmDialog(page, /empty trash|delete/i);
            await sleep(2000);
            await openSidebarAlbum(page, "Trash");
            await waitNoThumbnails(page, "trash not empty after emptying it");
        },
    },
    {
        name: "search_by_name_and_date",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            const search = async (query, suggestion, what) => {
                await escape(page);
                await page.getByRole("button", { name: "Search", exact: true }).first().click();
                // react-select input; its placeholder is a separate element.
                const input = page.locator("input[aria-label='Search']:visible").first();
                await input.click();
                await input.fill(query);
                await expectVisible(page.getByText(suggestion), what, 30000);
                await page.keyboard.press("Enter");
                await waitThumbnails(page, 1);
            };
            // (e2e-photo-1 may already be deleted by trash_restore_delete.)
            await search("e2e-photo", /^File name · \d+ memor/, "a file-name search suggestion");
            const month = new Date().toLocaleString("en-US", { month: "long" });
            await search(month, /^Date · \d+ memor/, `a date suggestion for "${month}"`);
            await escape(page);
        },
    },
    {
        name: "signup_second_user",
        async run(ctx) {
            const page = await ctx.newPage("user2");
            ctx.state.user2 = { page, recoveryKey: await signup(ctx, page, USER2) };
        },
    },
    {
        name: "share_album_with_user",
        deps: ["upload_photos_and_video", "signup_second_user"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSidebarAlbum(page, ALBUM);
            await shareButton(page).click();
            await btn(page, "Add viewers").click();
            await page.getByPlaceholder("Enter email address").fill(USER2.email);
            await page.locator("button[type=submit]", { hasText: "Add viewers" }).click();
            await expectVisible(page.getByText(USER2.email), "the viewer in the sharing list");
            await escape(page, 3);
            const p2 = ctx.state.user2.page;
            await p2.reload();
            await p2.waitForURL("**/gallery");
            await expectVisible(p2.getByText(ALBUM).first(), "the shared album for the second user", 60000);
            await p2.getByText(ALBUM).first().click();
            await waitThumbnails(p2, 1);
        },
    },
    {
        name: "public_link",
        deps: ["upload_photos_and_video"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSidebarAlbum(page, ALBUM);
            await shareButton(page).click();
            await btn(page, "Create public link").click();
            // The drawer then shows the full link (key in the fragment).
            await expectVisible(page.getByText("Public link enabled"), "the public link to be enabled");
            const text = await page.locator("body").innerText();
            const found = text.match(/(https?:\/\/)?[^\s/]+(\/[^\s?]*)?\?t=[A-Za-z0-9]+#[A-Za-z0-9]+/);
            if (!found) throw new Error("no public link shown after creating it");
            const link = found[0].startsWith("http") ? found[0] : `http://${found[0]}`;
            if (!/[?&]t=/.test(link) || !link.includes("#")) throw new Error(`unexpected public link "${link}"`);
            ctx.note(`public link: ${link.replace(/#.*/, "#<key>")}`);
            await escape(page, 3);
            // Open in a fresh, logged-out context, as a visitor would.
            const visitor = await ctx.newPage("visitor");
            let url = link;
            if (!link.startsWith(ctx.config.albums)) {
                const u = new URL(link);
                ctx.note(`link origin ${u.origin} is not the local albums app; opening ${ctx.config.albums} with the same token`);
                url = `${ctx.config.albums}/${u.search}${u.hash}`;
            }
            await visitor.goto(url);
            // The albums app has no .photo-list; any decrypted thumbnail will do.
            const deadline = Date.now() + 90000;
            while ((await visitor.locator("img[src^='blob:']").count()) < 2) {
                if (Date.now() > deadline) throw new Error("public album shows no decrypted thumbnails");
                await sleep(500);
            }
            await expectVisible(visitor.getByText(ALBUM), "the album name on the public page");
        },
    },
    {
        name: "legacy_trusted_contact",
        skip: "the Photos web app at the pinned commit has no Legacy (trusted contact) UI; it lives in the mobile apps and the Locker web app. Covered by the manual mobile checklist.",
        deps: ["signup", "signup_second_user"],
        async run() {},
    },
    {
        name: "active_sessions",
        deps: ["signup"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSettings(page);
            await page.getByRole("button", { name: /^Active sessions/ }).click();
            await reauthIfAsked(page, USER1.password);
            await expectVisible(page.getByText(/this device|current/i), "the current session in the list");
            await escape(page, 3);
        },
    },
    {
        name: "change_password",
        deps: ["signup"],
        async run(ctx) {
            const { page } = ctx.state.user1;
            await openSettings(page);
            await page.getByRole("button", { name: "Change password" }).click();
            await reauthIfAsked(page, USER1.password);
            await page.waitForURL("**/change-password");
            const inputs = page.locator("input[type=password]");
            await inputs.nth(0).waitFor();
            await inputs.nth(0).fill(NEW_PASSWORD);
            await inputs.nth(1).fill(NEW_PASSWORD);
            await page.locator("button[type=submit]").first().click();
            await page.waitForURL("**/gallery", { timeout: 120000 });
            await logout(page);
            await login(ctx, page, USER1, NEW_PASSWORD);
            USER1.password = NEW_PASSWORD;
        },
    },
    {
        name: "delete_account",
        deps: ["signup_second_user"],
        async run(ctx) {
            const page = ctx.state.user2.page;
            await openSettings(page);
            await page.getByRole("button", { name: /^Delete account/ }).click();
            const dialog = page.getByRole("dialog").last();
            await expectVisible(dialog, "the delete account dialog");
            const select = dialog.getByRole("combobox");
            if (await select.first().isVisible().catch(() => false)) {
                await select.first().click();
                await page.getByRole("option").first().click();
            }
            const feedback = dialog.locator("textarea:visible");
            if (await feedback.first().isVisible().catch(() => false)) await feedback.first().fill("e2e test account");
            await dialog.getByRole("button", { name: "Continue" }).click();
            // Summary of what will be deleted, with an "I understand" checkbox.
            await expectVisible(page.getByText(/I understand this deletes my data/), "the deletion summary");
            const understand = page.getByText(/I understand this deletes my data/);
            const del = page.getByRole("button", { name: "Delete Ente account" });
            for (let i = 0; i < 5 && !(await del.isEnabled()); i++) {
                await understand.click({ force: true });
                await sleep(300);
            }
            await del.click();
            await reauthIfAsked(page, USER2.password);
            await page.waitForURL((u) => !u.pathname.startsWith("/gallery"), { timeout: 60000 });
            // Logging in again must fail: the account is gone.
            await page.goto(`${ctx.config.web}/`);
            await page.locator("input[name=email]").fill(USER2.email);
            await btn(page, "Login").first().click();
            await sleep(3000);
            if (page.url().includes("/credentials")) throw new Error("deleted account still offers password login");
        },
    },
];
