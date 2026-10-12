/* Cairn site: small, dependency-free progressive enhancements. */
(function () {
  'use strict';

  var reduceMotion = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  /* ── mobile menu ───────────────────────────────────────── */
  var menuBtn = $('.menu-btn');
  var links = $('#nav-links');
  if (menuBtn && links) {
    var setMenu = function (open) {
      links.classList.toggle('open', open);
      menuBtn.setAttribute('aria-expanded', String(open));
    };
    menuBtn.addEventListener('click', function () {
      setMenu(menuBtn.getAttribute('aria-expanded') !== 'true');
    });
    links.addEventListener('click', function (e) {
      if (e.target.closest('a')) setMenu(false);
    });
    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape') setMenu(false);
    });
  }

  /* ── scroll reveal ─────────────────────────────────────── */
  var revealEls = $$('.reveal');
  if (!('IntersectionObserver' in window) || reduceMotion) {
    revealEls.forEach(function (el) { el.classList.add('in'); });
  } else {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.classList.add('in');
          io.unobserve(en.target);
        }
      });
    }, { rootMargin: '0px 0px -8% 0px', threshold: 0.08 });
    revealEls.forEach(function (el) { io.observe(el); });
  }

  /* ── pipeline: selecting a step highlights what it pulls in ── */
  var STEPS = [ // ALL_STEPS order, which is also a valid build order
    { id: 'download', name: 'Download OSM', deps: [] },
    { id: 'elevation', name: 'Elevation tiles', deps: [] },
    { id: 'basemap', name: 'Basemap', deps: ['download'] },
    { id: 'routes', name: 'Routes', deps: ['download'] },
    { id: 'bathymap', name: 'Bathymap', deps: [] },
    { id: 'terrain', name: 'Terrain RGB', deps: ['elevation'] },
    { id: 'vtiles', name: 'Valhalla tiles', deps: ['download', 'elevation'] },
    { id: 'package', name: 'Valhalla package', deps: ['vtiles'] }
  ];
  var byId = {};
  STEPS.forEach(function (s) { byId[s.id] = s; });

  var flow = $('#flow');
  var picks = $$('#picks .pick');
  var listItems = $$('#plan-list li');
  var note = $('#plan-note');

  function plan(target) {
    var set = {};
    (function visit(id) {
      if (set[id]) return;
      set[id] = true;
      byId[id].deps.forEach(visit);
    })(target);
    return STEPS.filter(function (s) { return set[s.id]; });
  }

  function select(target) {
    var steps = plan(target);
    var order = {};
    steps.forEach(function (s, i) { order[s.id] = i + 1; });

    picks.forEach(function (b) { b.setAttribute('aria-pressed', String(b.dataset.id === target)); });

    if (flow) {
      $$('.node', flow).forEach(function (n) {
        var id = n.dataset.id;
        var inPlan = !!order[id];
        n.classList.toggle('on', inPlan && id !== target);
        n.classList.toggle('target', id === target);
        n.classList.toggle('dim', !inPlan);
        var label = $('.badge text', n);
        if (label) label.textContent = inPlan ? String(order[id]) : '';
      });
      $$('.eg', flow).forEach(function (g) {
        var live = !!order[g.dataset.from] && !!order[g.dataset.to];
        $$('.edge', g).forEach(function (p) { p.classList.toggle('on', live); p.classList.toggle('dim', !live); });
        $$('.arrow', g).forEach(function (p) { p.classList.toggle('on', live); p.classList.toggle('dim', !live); });
      });
    }

    listItems.forEach(function (li) {
      var id = li.dataset.id;
      var inPlan = !!order[id];
      li.classList.toggle('on', inPlan && id !== target);
      li.classList.toggle('target', id === target);
      li.classList.toggle('dim', !inPlan);
      var n = $('.n', li);
      if (n) n.textContent = inPlan ? String(order[id]) : '';
    });

    if (note) {
      var name = byId[target].name;
      if (steps.length === 1) {
        note.innerHTML = 'Selecting <strong></strong> runs just that step. It has no prerequisites.';
        $('strong', note).textContent = name;
      } else {
        note.innerHTML = 'Selecting <strong></strong> runs <span class="n"></span> steps, in order: <span class="seq"></span>';
        $('strong', note).textContent = name;
        $('.n', note).textContent = String(steps.length);
        $('.seq', note).textContent = steps.map(function (s) { return s.name; }).join(' → ');
      }
    }
  }

  if (picks.length) {
    picks.forEach(function (b) { b.addEventListener('click', function () { select(b.dataset.id); }); });
    if (flow) {
      $$('.node', flow).forEach(function (n) {
        n.addEventListener('click', function () { select(n.dataset.id); });
      });
    }
    listItems.forEach(function (li) {
      li.addEventListener('click', function () { select(li.dataset.id); });
    });
    select('package');
  }

  /* ── map view switcher ─────────────────────────────────── */
  var stack = $('#view-stack');
  if (stack) {
    var imgs = $$('img', stack);
    var segBtns = $$('.seg button');
    segBtns.forEach(function (b) {
      b.addEventListener('click', function () {
        var i = Number(b.dataset.show);
        imgs.forEach(function (img, j) { img.classList.toggle('show', i === j); });
        segBtns.forEach(function (x) { x.setAttribute('aria-pressed', String(x === b)); });
      });
    });
  }

  /* ── copy button ───────────────────────────────────────── */
  var copyBtn = $('#copy-cli');
  var code = $('#cli-code');
  if (copyBtn && code) {
    var text = function () {
      return $$('.ln', code).map(function (ln) {
        var c = ln.cloneNode(true);
        $$('.p', c).forEach(function (p) { p.remove(); });
        return c.textContent.replace(/\s+$/, '');
      }).join('\n');
    };
    var fallback = function (t) {
      var ta = document.createElement('textarea');
      ta.value = t;
      ta.setAttribute('readonly', '');
      ta.style.cssText = 'position:fixed;left:-9999px;top:0';
      document.body.appendChild(ta);
      ta.select();
      var ok = false;
      try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
      document.body.removeChild(ta);
      return ok;
    };
    var label = $('span', copyBtn);
    var timer;
    var flash = function (msg, done) {
      label.textContent = msg;
      copyBtn.classList.toggle('done', done);
      clearTimeout(timer);
      timer = setTimeout(function () { label.textContent = 'Copy'; copyBtn.classList.remove('done'); }, 1800);
    };
    copyBtn.addEventListener('click', function () {
      var t = text();
      if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(t).then(function () { flash('Copied', true); },
          function () { flash(fallback(t) ? 'Copied' : 'Press Ctrl+C', true); });
      } else {
        flash(fallback(t) ? 'Copied' : 'Select and copy', true);
      }
    });
  }

  /* ── OS detection: highlight the matching download ─────── */
  (function () {
    var nav = window.navigator;
    var ua = (nav.userAgent || '').toLowerCase();
    var plat = ((nav.userAgentData && nav.userAgentData.platform) || nav.platform || '').toLowerCase();
    var os = null;
    if (/android|iphone|ipad|ipod|cros/.test(ua)) os = 'other';
    else if (/mac/.test(plat) || /mac os x|macintosh/.test(ua)) os = (nav.maxTouchPoints > 2) ? 'other' : 'mac';
    else if (/win/.test(plat) || /windows/.test(ua)) os = 'win';
    else if (/linux|x11/.test(plat) || /linux|x11/.test(ua)) os = 'linux';

    var noteEl = $('#os-note');
    if (os === 'mac' || os === 'linux') {
      var card = $('.dl[data-os="' + os + '"]');
      if (card) card.classList.add('is-match');
      if (noteEl) noteEl.textContent = 'Looks like you are on ' + (os === 'mac' ? 'macOS' : 'Linux') + '. Builds target macOS and Linux today; Windows has not been released.';
    } else if (os === 'win' && noteEl) {
      noteEl.textContent = 'Looks like you are on Windows. There is no Windows build yet: releases target macOS and Linux today.';
    }
  })();
})();
