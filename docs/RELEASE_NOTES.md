## What is in this release

Everything below landed since the previous release on `main`. Each item has a row in
`docs/TO-DO.md` (the `T-` numbers) saying exactly how it was checked.

### The AI agent

- **Replies stream in as they are written.** The agent's answer (or its question to you)
  appears word by word in the panel. It is only something to watch: the agent acts on a step
  only once the model has finished it and Ferrite has checked it (T-335).
- **Two new model providers:** Anthropic Claude, and OpenAI or any server with the same API
  (OpenRouter, Groq, vLLM, LM Studio, llama.cpp). Both are in Settings and in the live
  evaluation runner. A server on your own computer needs no key (T-277).
- Pressing Enter or Space counts as a click for the defense, a rejected site also blocks
  addresses typed without `https://`, and messages from a stopped run are ignored.

### The defense

- New sanitizer rules for claimed user approval, role-play framing and reworded "ignore your
  instructions" orders, with golden test cases (pattern set 3, T-327).
- The audit log now says exactly which entry breaks its hash chain, and how (T-219).

### Browsing

- **Fewer page errors on real sites.** Fixed in the browser's compatibility scripts and the
  engine, and checked on real sites by a CI job that loads 32 of them:
  - the container-query script no longer throws "text is null" (Reddit, X, Amazon,
    Cloudflare, React, Next.js, Tailwind, Vercel, Discord) (T-328);
  - SVG interfaces such as `SVGAElement` (nytimes.com, svelte.dev) (T-329);
  - `PublicKeyCredential`, saying there is no passkey device (amazon.com) (T-330);
  - new blank frames get the same compatibility scripts as their page (airbnb.com) (T-331);
  - the Web Animations API (YouTube) and `shadowRoot.getAnimations` (cloudflare.com).
- **GitHub no longer crashes the engine when its tab closes** (a WebGL clean-up bug) (T-333).
- **Google's "The operation is insecure" errors:** setting `document.domain` no longer cuts a
  page off from its own frames, and every security error names the property involved
  (T-267, T-321, T-323).
- **Security fix:** a page could read the document of a frame from another origin on the same
  site. It can no longer (T-322).
- **Sign-in for sites that use HTTP authentication:** a Sign in card with a hidden password
  field. The password never appears in a log, and the agent cannot fill the card (T-294).
- **File inputs open your system's own file dialog** (T-293).
- **Video and calls:** WebRTC data channels on macOS and Windows; a seek no longer drops a
  video track's end; WebM audio with Opus.

### Speed and the window

- The window wakes when the engine has something new instead of polling, and draws the page
  from one GPU texture; on macOS the page is read in the GPU's own byte order (T-320).
- Page dialogs and pickers ease in; a new tab grows into the tab strip.

### DevTools and logs

- The Network tab shows each request's **size and time** once it finishes (T-292).
- An unhandled promise rejection in the Console names the file and line it came from.
- **Windows now writes `ferrite.log`** (in `%LOCALAPPDATA%\Ferrite\logs`) (T-298).

### Still not done (honestly)

- Google sign-in has not been confirmed to work end to end (T-267).
- YouTube playback cannot be checked on CI: YouTube asks CI's machines to sign in ("confirm
  you're not a bot"). It needs a real computer.
- The Anthropic and OpenAI-compatible connections and streaming are tested against stand-in
  servers; nobody has run a live task through them yet.
- No status codes in the Network tab, no AVIF images, no input-method (IME) typing, and
  Cloudflare's "Just a moment…" check does not finish on some sites (T-292, T-332, T-294).
- The builds are unsigned prototypes.
