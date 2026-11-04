# Casting to a TV

Albums can be shown on a TV or Chromecast as a slideshow. The TV runs the cast
page your server serves at `<public URL>/cast`. Photos stay end-to-end
encrypted: the phone hands the TV only that album's key, encrypted for the TV.

## Pair with a code (works everywhere)

1. On the TV (or any browser), open `<public URL>/cast`. It shows a code.
2. In the app: open an album, **Cast**, **Pair using code**, enter the code.

Nothing else is needed. This is the only option until you set up a Cast
receiver (below).

## Chromecast auto-pairing (optional)

"Auto pair" finds Chromecasts on your Wi-Fi and starts the cast page on them.
Google only lets a Chromecast open pages registered as a *Cast receiver app*.
Ente's apps use Ente's own receiver, which loads Ente's website and talks to
Ente's servers, so Photovault does not use it: you register your own.

1. Your server needs an `https://` public address that the Chromecast can
   reach (Google requires HTTPS for receivers).
2. Sign in to the [Google Cast SDK Developer Console](https://cast.google.com/publish)
   (one-time USD 5 registration fee).
3. **Add new application** → **Custom Receiver**. Name: `Photovault`.
   Receiver Application URL: `<public URL>/cast`, for example
   `https://photos.example.com/cast`. Copy the 8-character **Application ID**.
4. While the app is unpublished, it only runs on Chromecasts you add under
   **Cast Receiver Devices** (by serial number; wait about 15 minutes, then
   reboot the Chromecast). Publishing it removes that limit; the page only
   shows a pairing screen, so publishing exposes nothing.
5. Set the id on the server and restart:

   ```yaml
   environment:
     PHOTOVAULT_CAST_APP_ID: 1A2B3C4D
   ```

The server announces it as `castAppId` in `/photovault/info`; the app picks it
up on its next address check (app start or network change) and shows
**Auto pair**. A wrong value stops the server at startup with a fix line.

App builders can bake a receiver into the APK instead with
`--dart-define=PHOTOVAULT_CAST_APP_ID=<id>`; the server's value wins.
