/* documentation.js — renders the repository's markdown in the browser.
   The docs live beside this page (GitHub Pages serves docs/ as the site
   root and .nojekyll keeps the .md files raw), so a page is one fetch away
   and nothing is copied. Routing is the hash: `#guides/http-api.md`, or
   `#architecture.md:5-retrieval-pipeline` to land on a heading. */
(function () {
  'use strict';

  var ROOT = new URL('.', window.location.href);
  var REPO_ROOT = new URL('..', ROOT);
  var GITHUB = 'https://github.com/Falconiere/comemory/blob/main/';
  var INDEX = 'README.md';
  var SITE_TITLE = 'comemory docs';

  var nav = document.getElementById('docs-nav');
  var crumbs = document.getElementById('docs-crumbs');
  var state = document.getElementById('docs-state');
  var toc = document.getElementById('docs-toc');
  var body = document.getElementById('docs-body');
  var cache = new Map();
  var current = { path: null, frag: null };

  function escapeHtml(text) {
    return String(text).replace(/[&<>"']/g, function (ch) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch];
    });
  }

  /* ---- routing ---------------------------------------------------------- */

  function parseHash() {
    var raw;
    try { raw = decodeURIComponent(window.location.hash.replace(/^#/, '')); } catch (error) { raw = ''; }
    if (!raw) return { path: INDEX, frag: null };
    var at = raw.indexOf(':');
    var path = at === -1 ? raw : raw.slice(0, at);
    var frag = at === -1 ? null : raw.slice(at + 1);
    if (!/\.md$/.test(path)) return { path: INDEX, frag: null };
    return { path: normalize(path), frag: frag || null };
  }

  function normalize(path) {
    var url = new URL(path, ROOT);
    if (url.pathname.indexOf(ROOT.pathname) !== 0) return INDEX;
    return url.pathname.slice(ROOT.pathname.length);
  }

  function hashFor(path, frag) {
    return '#' + path + (frag ? ':' + frag : '');
  }

  /* ---- links ------------------------------------------------------------ */

  /* Every relative link in a page is rewritten once: a .md inside docs/
     stays in the reader, anything else in the repository opens on GitHub,
     and an in-page anchor keeps the current path so the router can scroll. */
  function rewriteLink(anchor, docPath) {
    var href = anchor.getAttribute('href');
    if (!href) return;
    if (href.charAt(0) === '#') {
      anchor.setAttribute('href', hashFor(docPath, href.slice(1)));
      return;
    }
    var base = new URL(docPath, ROOT);
    var url;
    try { url = new URL(href, base); } catch (error) { return; }
    if (url.origin !== ROOT.origin) {
      anchor.setAttribute('rel', 'noopener');
      return;
    }
    if (url.pathname.indexOf(ROOT.pathname) === 0) {
      var rel = url.pathname.slice(ROOT.pathname.length);
      if (/\.md$/.test(rel)) {
        anchor.setAttribute('href', hashFor(rel, url.hash ? url.hash.slice(1) : null));
      } else {
        /* any other file beside a doc resolves against the doc's own path,
           not against this page's URL */
        anchor.setAttribute('href', url.href);
      }
      return;
    }
    if (url.pathname.indexOf(REPO_ROOT.pathname) === 0) {
      anchor.setAttribute('href', GITHUB + url.pathname.slice(REPO_ROOT.pathname.length) + url.hash);
      anchor.setAttribute('rel', 'noopener');
    }
  }

  /* ---- headings --------------------------------------------------------- */

  /* GitHub's slug: lowercase, drop punctuation, spaces to hyphens, and a
     numeric suffix on a repeat, so links written against GitHub resolve. */
  function slugify(text, seen) {
    var slug = text.trim().toLowerCase().replace(/[^\w\- ]+/g, '').replace(/ /g, '-');
    var unique = slug;
    var n = 1;
    while (seen.has(unique)) { unique = slug + '-' + n; n += 1; }
    seen.add(unique);
    return unique;
  }

  function decorateHeadings(container, docPath) {
    var seen = new Set();
    var entries = [];
    var headings = container.querySelectorAll('h1, h2, h3, h4, h5, h6');
    Array.prototype.forEach.call(headings, function (heading) {
      var id = slugify(heading.textContent, seen);
      heading.id = id;
      var depth = Number(heading.tagName.charAt(1));
      if (depth === 2 || depth === 3) entries.push({ id: id, depth: depth, text: heading.textContent });
      if (depth > 1) {
        var link = document.createElement('a');
        link.className = 'anchor';
        link.href = hashFor(docPath, id);
        link.setAttribute('aria-label', 'Link to this section');
        link.textContent = '§';
        heading.appendChild(link);
      }
    });
    return entries;
  }

  function renderToc(entries) {
    toc.innerHTML = '';
    if (entries.length < 2) { toc.hidden = true; return; }
    var label = document.createElement('p');
    label.className = 'label';
    label.textContent = 'on this page';
    var list = document.createElement('ol');
    entries.forEach(function (entry) {
      var item = document.createElement('li');
      item.setAttribute('data-depth', String(entry.depth));
      var link = document.createElement('a');
      link.href = hashFor(current.path, entry.id);
      link.textContent = entry.text;
      item.appendChild(link);
      list.appendChild(item);
    });
    toc.appendChild(label);
    toc.appendChild(list);
    toc.hidden = false;
  }

  /* ---- states ----------------------------------------------------------- */

  function showLoading(path) {
    state.innerHTML = '';
    var line = document.createElement('p');
    line.className = 'meta';
    line.textContent = 'loading · ' + path;
    state.appendChild(line);
  }

  function showError(path, status) {
    state.innerHTML = '';
    var banner = document.createElement('div');
    banner.className = 'banner';
    banner.setAttribute('data-status', 'incident');
    banner.setAttribute('role', 'alert');
    var dot = document.createElement('span');
    dot.className = 'dot';
    dot.setAttribute('aria-hidden', 'true');
    var text = document.createElement('p');
    var tag = document.createElement('span');
    tag.className = 'tag';
    tag.textContent = 'incident · ' + status;
    text.appendChild(tag);
    text.appendChild(document.createTextNode('Could not load ' + path + '. The page still exists on GitHub.'));
    var action = document.createElement('a');
    action.className = 'btn btn-ghost';
    action.href = GITHUB + 'docs/' + path;
    action.textContent = 'Open on GitHub';
    banner.appendChild(dot);
    banner.appendChild(text);
    banner.appendChild(action);
    state.appendChild(banner);
  }

  function clearState() { state.innerHTML = ''; }

  /* ---- fetch + render --------------------------------------------------- */

  function fetchDoc(path) {
    if (cache.has(path)) return Promise.resolve(cache.get(path));
    return fetch(new URL(path, ROOT).href, { cache: 'no-cache' }).then(function (response) {
      if (!response.ok) throw new Error(String(response.status));
      return response.text();
    }).then(function (text) {
      cache.set(path, text);
      return text;
    });
  }

  function renderCrumbs(path) {
    crumbs.innerHTML = '';
    var parts = path.split('/');
    var home = document.createElement('a');
    home.className = 'meta';
    home.href = hashFor(INDEX, null);
    home.textContent = 'docs';
    crumbs.appendChild(home);
    parts.forEach(function (part, index) {
      var sep = document.createElement('span');
      sep.className = 'meta';
      sep.textContent = '/';
      crumbs.appendChild(sep);
      var node = document.createElement('span');
      node.className = 'meta';
      node.textContent = part;
      if (index === parts.length - 1) node.style.color = 'var(--text)';
      crumbs.appendChild(node);
    });
    var edit = document.createElement('a');
    edit.className = 'meta';
    edit.href = GITHUB + 'docs/' + path;
    edit.setAttribute('rel', 'noopener');
    edit.textContent = '· source ↗';
    crumbs.appendChild(edit);
  }

  function renderDoc(path, markdown) {
    body.innerHTML = window.marked.parse(markdown);
    Array.prototype.forEach.call(body.querySelectorAll('a[href]'), function (anchor) {
      rewriteLink(anchor, path);
    });
    var entries = decorateHeadings(body, path);
    renderToc(entries);
    var first = body.querySelector('h1');
    document.title = (first && path !== INDEX ? first.textContent.trim() + ' · ' : '') + SITE_TITLE;
  }

  /* Landing on a route is instant; a click within the same page scrolls
     smoothly. Web fonts swap after first paint and move every line below
     them, so a landing is repeated once the fonts are in. */
  function scrollTo(frag, behavior) {
    function land() {
      var target = frag ? document.getElementById(frag) : null;
      if (target) target.scrollIntoView({ block: 'start', behavior: behavior });
      else window.scrollTo({ top: 0, behavior: behavior });
    }
    land();
    if (behavior === 'instant' && document.fonts && document.fonts.ready) document.fonts.ready.then(land);
  }

  function markCurrent(path) {
    Array.prototype.forEach.call(nav.querySelectorAll('a'), function (anchor) {
      if (anchor.getAttribute('href') === hashFor(path, null)) anchor.setAttribute('aria-current', 'page');
      else anchor.removeAttribute('aria-current');
    });
  }

  function route() {
    var next = parseHash();
    var samePage = next.path === current.path;
    current = next;
    markCurrent(next.path);
    if (samePage) { scrollTo(next.frag, 'smooth'); return; }
    renderCrumbs(next.path);
    showLoading(next.path);
    fetchDoc(next.path).then(function (markdown) {
      if (current.path !== next.path) return;
      clearState();
      renderDoc(next.path, markdown);
      scrollTo(next.frag, 'instant');
    }).catch(function (error) {
      if (current.path !== next.path) return;
      body.innerHTML = '';
      toc.hidden = true;
      showError(next.path, error.message || 'failed');
    });
  }

  /* ---- sidebar ---------------------------------------------------------- */

  /* The sidebar is docs/README.md itself: every h2 becomes a group, every
     linked list item under it an entry. Change the index, change the nav. */
  function buildNav(markdown) {
    var scratch = document.createElement('div');
    scratch.innerHTML = window.marked.parse(markdown);
    nav.innerHTML = '';
    var group = null;
    var list = null;
    Array.prototype.forEach.call(scratch.children, function (node) {
      if (node.tagName === 'H2') {
        group = document.createElement('div');
        group.className = 'group';
        var label = document.createElement('p');
        label.className = 'label';
        label.textContent = node.textContent;
        group.appendChild(label);
        list = document.createElement('ul');
        group.appendChild(list);
        return;
      }
      if (node.tagName !== 'UL' || !list) return;
      Array.prototype.forEach.call(node.querySelectorAll(':scope > li'), function (item) {
        var anchor = item.querySelector('a[href]');
        if (!anchor) return;
        rewriteLink(anchor, INDEX);
        if (anchor.getAttribute('href').charAt(0) !== '#') return;
        var entry = document.createElement('li');
        var link = document.createElement('a');
        link.href = anchor.getAttribute('href');
        link.textContent = anchor.textContent;
        entry.appendChild(link);
        list.appendChild(entry);
      });
      /* a heading with no linked list under it is prose, not a group */
      if (list.children.length && !group.parentNode) nav.appendChild(group);
    });
    markCurrent(current.path || INDEX);
  }

  /* The sidebar is a <details>: open on a wide screen, where its summary is
     hidden, and closed on a phone, where 23 links would precede the page. */
  function foldSidebar() {
    var details = document.querySelector('.docs-side details');
    var narrow = window.matchMedia('(max-width: 53.74rem)');
    if (!details) return;
    function apply() { details.open = !narrow.matches; }
    apply();
    if (narrow.addEventListener) narrow.addEventListener('change', apply);
  }

  function start() {
    foldSidebar();
    if (!window.marked) {
      showError('the markdown renderer', 'script blocked');
      return;
    }
    /* Raw HTML in a page is shown as text, never parsed: the docs are plain
       markdown (every angle bracket in them sits inside a code span), so the
       only thing passthrough could add is a script. Escaping needs no
       sanitizer library and keeps the reader a markdown viewer. */
    window.marked.use({
      gfm: true,
      breaks: false,
      renderer: {
        html: function (token) { return escapeHtml(token.text); }
      }
    });
    fetchDoc(INDEX).then(buildNav).catch(function () {
      nav.innerHTML = '';
      var line = document.createElement('p');
      line.className = 'meta';
      line.textContent = 'index unavailable · README.md';
      nav.appendChild(line);
    });
    window.addEventListener('hashchange', route);
    route();
  }

  start();
})();
