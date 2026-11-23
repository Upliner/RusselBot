// The page of RusselBot's web interface. Plain JavaScript, no framework,
// nothing loaded from elsewhere.
//
// Everything the page does is a command of the bot, sent as the control
// socket takes it: the command's name and its arguments separated by tabs,
// with backslash, tab, line feed and carriage return written as \\, \t, \n
// and \r. Answers and listings come back in the same form. What the avatar
// hears and where it is arrive over one event stream.
//
// Text from the grid is only ever put on the page as text (textContent):
// nothing here builds markup from a string.

'use strict';

// The theme, before anything is painted: this script is loaded in <head>.
(function () {
  var theme = null;
  try { theme = window.localStorage.getItem('theme'); } catch (e) { /* no storage: the browser's theme */ }
  if (theme === 'light' || theme === 'dark') {
    document.documentElement.setAttribute('data-theme', theme);
  }
})();

(function () {
  var TABS = ['world', 'chat', 'im', 'inventory', 'console', 'settings'];
  var NEARBY_EVERY = 15000;
  var FRIENDS_EVERY = 60000;
  var MAX_CHAT_LINES = 1000;
  var UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  var NUMBER = /^-?(\d+\.?\d*|\.\d+)$/;

  var state;

  function freshState() {
    return {
      tab: 'world',
      session: 'open',        // 'open' (no password) or 'in' (logged in)
      lost: false,
      status: null,           // the status text's lines as an object
      chat: [],               // records of local chat
      chatFollow: true,       // the chat view is at its end
      convs: {},              // conversations by key
      conv: null,             // the key of the one shown
      pendingSent: null,      // a first message sent by name: {key, text}
      nearby: null,
      nearbyBusy: false,
      friends: null,
      friendsBusy: false,
      inventory: { root: null, attachedOnly: false, attached: null, started: false },
      history: [],            // console lines sent
      historyAt: -1
    };
  }

  // --------------------------------------------------------------- helpers

  function $(id) { return document.getElementById(id); }

  // A new element with a class and a text. The text is text, whatever it holds.
  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) { node.className = cls; }
    if (text !== undefined && text !== null) { node.textContent = text; }
    return node;
  }

  function clear(node) {
    while (node.firstChild) { node.removeChild(node.firstChild); }
  }

  function show(node, on) { node.hidden = !on; }

  function escapeField(text) {
    return String(text).replace(/\\/g, '\\\\').replace(/\t/g, '\\t').replace(/\n/g, '\\n').replace(/\r/g, '\\r');
  }

  function unescapeField(text) {
    var out = '';
    for (var i = 0; i < text.length; i++) {
      var c = text[i];
      if (c !== '\\' || i + 1 >= text.length) { out += c; continue; }
      var next = text[++i];
      if (next === '\\') { out += '\\'; }
      else if (next === 't') { out += '\t'; }
      else if (next === 'n') { out += '\n'; }
      else if (next === 'r') { out += '\r'; }
      else { out += '\\' + next; }
    }
    return out;
  }

  // The fields of the lines of a listing.
  function listing(text) {
    if (!text) { return []; }
    return text.split('\n').filter(function (line) { return line !== ''; }).map(function (line) {
      return line.split('\t').map(unescapeField);
    });
  }

  // The names of an inventory path, which writes a slash inside a name as \/.
  // The path's fields were unescaped once already, so \\ is one backslash here.
  function pathNames(path) {
    var names = [];
    var name = '';
    for (var i = 0; i < path.length; i++) {
      var c = path[i];
      if (c === '\\' && i + 1 < path.length) { name += path[++i]; }
      else if (c === '/') { names.push(name); name = ''; }
      else { name += c; }
    }
    if (name !== '') { names.push(name); }
    return names;
  }

  function two(n) { return (n < 10 ? '0' : '') + n; }

  function clock(date, seconds) {
    return two(date.getHours()) + ':' + two(date.getMinutes()) + (seconds ? ':' + two(date.getSeconds()) : '');
  }

  // The time of a message: the hour, and the day when it is not today.
  function timeLabel(epoch) {
    var date = new Date(epoch * 1000);
    var now = new Date();
    var sameDay = date.getFullYear() === now.getFullYear() && date.getMonth() === now.getMonth() && date.getDate() === now.getDate();
    return (sameDay ? '' : two(date.getMonth() + 1) + '-' + two(date.getDate()) + ' ') + clock(date, false);
  }

  function timeNode(epoch) {
    var node = el('span', 'time', timeLabel(epoch));
    node.title = new Date(epoch * 1000).toLocaleString();
    return node;
  }

  // ---------------------------------------------------------------- notices

  // What became of the operator's commands, visible on every tab. One per
  // control: an error stays until it is dismissed or the control is used
  // again; an answer goes away by itself.
  var notices = {};

  function dismiss(key) {
    var notice = notices[key];
    if (!notice) { return; }
    if (notice.timer) { window.clearTimeout(notice.timer); }
    if (notice.node.parentNode) { notice.node.parentNode.removeChild(notice.node); }
    delete notices[key];
  }

  function notify(key, what, text, isError) {
    dismiss(key);
    var node = el('div', 'notice' + (isError ? ' error' : ''));
    node.setAttribute('role', isError ? 'alert' : 'status');
    node.appendChild(el('span', 'what', what));
    node.appendChild(el('span', 'text', text));
    var close = el('button', null, '×');
    close.type = 'button';
    close.title = 'Dismiss';
    close.addEventListener('click', function () { dismiss(key); });
    node.appendChild(close);
    $('notices').appendChild(node);
    var notice = { node: node, timer: null };
    if (!isError) { notice.timer = window.setTimeout(function () { dismiss(key); }, 6000); }
    notices[key] = notice;
  }

  function dismissAll() {
    Object.keys(notices).forEach(dismiss);
  }

  // --------------------------------------------------------------- the bot

  function post(path, body, own) {
    var headers = { 'X-Rslbot': '1' };
    if (own) { headers['X-Rslbot-Own'] = '1'; }
    return window.fetch(path, { method: 'POST', headers: headers, body: body, cache: 'no-store', credentials: 'same-origin' });
  }

  // The answer line of a command: ok with a text or without, or an error.
  function decodeAnswer(line) {
    line = line.replace(/\r?\n$/, '');
    var tab = line.indexOf('\t');
    var head = tab < 0 ? line : line.slice(0, tab);
    var text = tab < 0 ? '' : unescapeField(line.slice(tab + 1));
    return { ok: head === 'ok', text: text };
  }

  function answerOf(response) {
    if (response.status === 401) {
      needLogin();
      return { ok: false, text: 'the session has ended; log in again' };
    }
    return response.text().then(function (text) {
      reached();
      if (response.status !== 200) {
        return { ok: false, text: text.trim() || 'the bot answered ' + response.status };
      }
      return decodeAnswer(text);
    });
  }

  function unreachable() {
    setLost(true);
    return { ok: false, text: 'the bot cannot be reached; nothing was sent' };
  }

  // Carries out a command given as its name and arguments. `own` marks
  // one the page sends by itself, to keep a list up to date.
  function run(args, own) {
    return post('/api/command', args.map(escapeField).join('\t'), own).then(answerOf, unreachable);
  }

  // Carries out a line as it was typed in the console.
  function runLine(line) {
    return post('/api/console', line, false).then(answerOf, unreachable);
  }

  // Carries out the command of a control: the control shows that it is
  // under way and does not send it twice; its answer or its error is shown.
  function act(button, key, what, args, quiet) {
    if (button.disabled) { return Promise.resolve(null); }
    dismiss(key);
    button.disabled = true;
    button.classList.add('busy');
    return run(args, false).then(function (result) {
      button.disabled = false;
      button.classList.remove('busy');
      if (!result.ok) { notify(key, what, result.text, true); }
      else if (!quiet) { notify(key, what, result.text || 'done', false); }
      return result;
    });
  }

  // ------------------------------------------------------- losing the bot

  var stream = null;
  var lastId = null;
  var retryIn = 1000;
  var retryTimer = null;
  var replaying = false;  // what arrives is from before the page was opened

  function setLost(lost) {
    if (state.lost === lost) { return; }
    state.lost = lost;
    show($('lost'), lost);
    if (lost && !stream && retryTimer === null) { scheduleRetry(); }
  }

  // A command was answered: the bot is there, if the stream says so too.
  function reached() {
    if (state.lost && stream && stream.readyState === 1) { found(); }
  }

  function found() {
    var was = state.lost;
    retryIn = 1000;
    setLost(false);
    if (was) { refreshOwn(); }
  }

  function scheduleRetry() {
    if (retryTimer !== null) { return; }
    retryTimer = window.setTimeout(function () {
      retryTimer = null;
      reconnect();
    }, retryIn);
    retryIn = Math.min(retryIn * 2, 30000);
  }

  function reconnect() {
    window.fetch('/api/session', { cache: 'no-store', credentials: 'same-origin' }).then(function (response) {
      return response.text();
    }).then(function (text) {
      if (text.trim() === 'login') { needLogin(); }
      else { openStream(); }
    }, function () {
      scheduleRetry();
    });
  }

  function fields(event) {
    return event.data.split('\t').map(unescapeField);
  }

  function openStream() {
    if (stream) { stream.close(); }
    var url = '/api/events' + (lastId ? '?last=' + encodeURIComponent(lastId) : '');
    var source = new window.EventSource(url);
    stream = source;
    source.addEventListener('reset', function () {
      // What follows is everything the bot kept: start over.
      replaying = true;
      dropMessages();
    });
    source.addEventListener('chat', function (event) {
      if (event.lastEventId) { lastId = event.lastEventId; }
      addChat(record(fields(event)));
    });
    source.addEventListener('im', function (event) {
      if (event.lastEventId) { lastId = event.lastEventId; }
      addIm(record(fields(event)));
    });
    source.addEventListener('status', function (event) {
      // The status ends what a page is sent when it opens its stream.
      replaying = false;
      setStatus(unescapeField(event.data));
      found();
    });
    source.onerror = function () {
      // The browser would try again at a pace of its own, and without
      // saying whether the session is gone: the page does it itself.
      source.close();
      if (stream === source) { stream = null; }
      setLost(true);
      scheduleRetry();
    };
  }

  function closeStream() {
    if (stream) { stream.close(); stream = null; }
    if (retryTimer !== null) { window.clearTimeout(retryTimer); retryTimer = null; }
  }

  // A message as the event stream gives it.
  function record(f) {
    return {
      time: parseInt(f[0], 10) || 0,
      kind: f[1] || '',
      avatarId: (f[2] || '').toLowerCase(),
      avatarName: f[3] || '',
      displayName: f[4] || '',
      objectId: (f[5] || '').toLowerCase(),
      objectName: f[6] || '',
      offline: f[7] === '1',
      text: f[8] || ''
    };
  }

  // The lists the page keeps up to date by itself, for the tab that is shown.
  function refreshOwn() {
    if (state.tab === 'chat') { refreshNearby(true); }
    if (state.tab === 'im') { refreshFriends(true); }
  }

  // ----------------------------------------------------------------- login

  function showLogin(message) {
    show($('app'), false);
    show($('login'), true);
    var note = $('login-message');
    note.textContent = message || '';
    show(note, !!message);
    $('login-password').value = '';
    $('login-password').focus();
  }

  // The session is gone: nothing of what was shown stays on screen.
  function needLogin() {
    if (!$('login').hidden) { return; }
    closeStream();
    lastId = null;
    retryIn = 1000;
    dismissAll();
    state = freshState();
    renderEverything();
    show($('lost'), false);
    showLogin('');
  }

  function enter(mode) {
    state.session = mode;
    show($('login'), false);
    show($('app'), true);
    show($('logout-panel'), mode === 'in');
    route();
    openStream();
  }

  function begin() {
    window.fetch('/api/session', { cache: 'no-store', credentials: 'same-origin' }).then(function (response) {
      return response.text().then(function (text) {
        text = text.trim();
        if (response.status !== 200) {
          // Refused before anything else, as under a name the bot does not
          // answer to: say what the bot said.
          show($('app'), false);
          show($('login'), false);
          var refusal = el('p', 'login error', text || 'The bot answered ' + response.status + '.');
          document.body.appendChild(refusal);
        } else if (text === 'login') {
          showLogin('');
        } else {
          enter(text === 'in' ? 'in' : 'open');
        }
      });
    }, function () {
      // Not there right now: show the page and keep trying.
      show($('app'), true);
      route();
      setLost(true);
    });
  }

  function wireLogin() {
    $('login').addEventListener('submit', function (event) {
      event.preventDefault();
      var button = $('login-button');
      if (button.disabled) { return; }
      var password = $('login-password').value;
      button.disabled = true;
      post('/api/login', password, false).then(function (response) {
        button.disabled = false;
        if (response.status === 204) { enter('in'); }
        else if (response.status === 403) { showLogin('The password is wrong.'); }
        else if (response.status === 429) { showLogin('Too many attempts. Try again in a minute.'); }
        else { showLogin('The bot answered ' + response.status + '.'); }
      }, function () {
        button.disabled = false;
        showLogin('The bot cannot be reached.');
      });
    });
    $('logout-button').addEventListener('click', function () {
      post('/api/logout', '', false).then(needLogin, needLogin);
    });
  }

  // ------------------------------------------------------------------ tabs

  function tabOfAddress() {
    var name = window.location.hash.replace(/^#/, '');
    return TABS.indexOf(name) >= 0 ? name : 'world';
  }

  function route() {
    var tab = tabOfAddress();
    state.tab = tab;
    TABS.forEach(function (name) { show($('tab-' + name), name === tab); });
    var links = $('tabs').querySelectorAll('a');
    for (var i = 0; i < links.length; i++) {
      var current = links[i].getAttribute('data-tab') === tab;
      links[i].classList.toggle('current', current);
      if (current) { links[i].setAttribute('aria-current', 'page'); } else { links[i].removeAttribute('aria-current'); }
    }
    // Behind the login box nothing is asked of the bot; `enter` routes
    // again once the page is let in.
    if ($('app').hidden) { return; }
    if (tab === 'chat') {
      show($('mark-chat'), false);
      if (state.chatFollow) { scrollToEnd($('chat-log')); }
      refreshNearby(true);
    } else if (tab === 'im') {
      seen(state.conv);
      renderConversations();
      scrollToEnd($('im-log'));
      refreshFriends(true);
    } else if (tab === 'inventory') {
      if (!state.inventory.started) { state.inventory.started = true; reloadInventory(); }
    }
  }

  // ---------------------------------------------------------------- status

  function setStatus(text) {
    var status = {};
    text.split('\n').forEach(function (line) {
      var colon = line.indexOf(': ');
      if (colon > 0) { status[line.slice(0, colon)] = line.slice(colon + 2); }
    });
    var before = state.status ? state.status.region : undefined;
    state.status = status;
    renderStatus();
    // Another region has other avatars in it.
    if (status.region !== before && state.tab === 'chat') { refreshNearby(true); }
  }

  function statusItem(name, value, cls) {
    var item = el('span', 'item');
    item.appendChild(el('span', 'name', name));
    item.appendChild(el('span', cls || 'value', value));
    return item;
  }

  function seatText(status) {
    var seat = status.seat || 'unknown';
    var name = status['seat name'] ? unescapeField(status['seat name']) : '';
    if (seat === 'none') { return 'not seated'; }
    if (seat === 'unknown') { return 'not known yet'; }
    if (name) { return name + (seat === 'target' ? ' (the held seat)' : ''); }
    if (seat === 'target') { return 'the held seat'; }
    if (seat === 'other') { return 'something unnamed'; }
    return seat;
  }

  function renderStatus() {
    var panels = document.querySelectorAll('[data-status]');
    for (var i = 0; i < panels.length; i++) {
      var panel = panels[i];
      clear(panel);
      var status = state.status;
      if (!status) {
        panel.appendChild(statusItem('Avatar', 'not known yet'));
      } else if (status.connected !== 'yes') {
        panel.appendChild(statusItem('Avatar', 'not connected', 'off'));
        if (status.mode) { panel.appendChild(statusItem('Mode', status.mode)); }
      } else {
        panel.appendChild(statusItem('Region', status.region || '?'));
        panel.appendChild(statusItem('Position', status.position || '?'));
        panel.appendChild(statusItem('Seat', seatText(status)));
        panel.appendChild(statusItem('Mode', status.mode || '?'));
      }
    }
  }

  // ----------------------------------------------------------------- world

  // A coordinate box: its number, '' when empty and allowed to be, or null
  // when it does not hold what it should, in which case the box is marked.
  function coordinate(input, required) {
    var text = input.value.trim();
    var good = text === '' ? !required : NUMBER.test(text);
    input.classList.toggle('bad', !good);
    return good ? text : null;
  }

  function wireWorld() {
    $('tp-form').addEventListener('submit', function (event) {
      event.preventDefault();
      var x = coordinate($('tp-x'), true);
      var y = coordinate($('tp-y'), true);
      var z = coordinate($('tp-z'), false);
      if (x === null || y === null || z === null) { return; }
      var region = $('tp-region').value.trim();
      var args = ['tp', x, y];
      if (z !== '') { args.push(z); }
      if (region !== '') { args.push(region); }
      act($('tp-button'), 'tp', 'Teleport', args, false);
    });
    $('stand-button').addEventListener('click', function () {
      act($('stand-button'), 'stand', 'Stand up', ['stand'], false);
    });
    $('avatars-form').addEventListener('submit', function (event) {
      event.preventDefault();
      var args = ['scan', 'avatars'];
      var distance = $('avatars-distance').value.trim();
      if (distance !== '') { args.push(distance); }
      act($('avatars-button'), 'avatars', 'Scan avatars', args, true).then(function (result) {
        if (result && result.ok) { renderAvatars(listing(result.text)); }
      });
    });
    $('objects-form').addEventListener('submit', function (event) {
      event.preventDefault();
      var args = ['scan', '--position'];
      var distance = $('objects-distance').value.trim();
      var name = $('objects-name').value.trim();
      if (distance !== '') { args.push(distance); }
      if (name !== '') { args.push('-n', name); }
      act($('objects-button'), 'objects', 'Scan objects', args, true).then(function (result) {
        if (result && result.ok) { renderObjects(listing(result.text)); }
      });
    });
  }

  function unknown(text) { return text === '?' || text === '' || text === undefined; }

  // The arguments of the teleport to a position of a listing, `x,y,z`:
  // without the height when the listing does not know it.
  function teleportTo(position) {
    var parts = position.split(',');
    if (parts.length < 2 || unknown(parts[0]) || unknown(parts[1])) { return null; }
    var args = ['tp', parts[0], parts[1]];
    if (parts.length > 2 && !unknown(parts[2])) { args.push(parts[2]); }
    return args;
  }

  function rowButton(label, onClick) {
    var button = el('button', null, label);
    button.type = 'button';
    button.addEventListener('click', function () { onClick(button); });
    return button;
  }

  function made(node, count, what) {
    node.textContent = 'Scanned at ' + clock(new Date(), true) + ': ' +
      (count === 0 ? 'nothing was found.' : count + ' ' + what + (count === 1 ? '' : 's') + '.');
    show(node, true);
  }

  // The name of an avatar as it is shown: with its display name where
  // that says more.
  function avatarLabel(name, display) {
    var legacy = unknown(name) ? '' : name;
    var shown = unknown(display) ? '' : display;
    if (legacy === '' && shown === '') { return ''; }
    if (legacy === '') { return shown; }
    if (shown === '' || sameName(legacy, shown)) { return legacy; }
    return shown + ' (' + legacy + ')';
  }

  function sameName(legacy, display) {
    var first = legacy.replace(/ Resident$/, '');
    return display.toLowerCase() === legacy.toLowerCase() || display.toLowerCase() === first.toLowerCase();
  }

  function renderAvatars(lines) {
    var list = $('avatars-list');
    clear(list);
    made($('avatars-made'), lines.length, 'avatar');
    lines.forEach(function (f) {
      // UUID, distance, position, name, display name.
      var id = f[0], distance = f[1], position = f[2] || '';
      var label = avatarLabel(f[3], f[4]) || (unknown(id) ? 'Someone (hidden)' : id);
      var row = el('li');
      var who = el('div', 'who');
      who.appendChild(el('div', 'title', label));
      who.appendChild(el('div', 'more', (unknown(distance) ? 'distance unknown' : distance + ' m') + ' · at ' + position));
      row.appendChild(who);
      var acts = el('div', 'acts');
      var to = teleportTo(position);
      if (to) {
        acts.appendChild(rowButton('Teleport', function (button) {
          act(button, 'av-tp:' + id + position, 'Teleport to ' + label, to, false);
        }));
      }
      if (UUID.test(id)) {
        acts.appendChild(rowButton('IM', function () {
          openAvatar(id, unknown(f[3]) ? '' : f[3], unknown(f[4]) ? '' : f[4]);
          window.location.hash = '#im';
        }));
      }
      row.appendChild(acts);
      list.appendChild(row);
    });
  }

  function renderObjects(lines) {
    var list = $('objects-list');
    clear(list);
    made($('objects-made'), lines.length, 'object');
    lines.forEach(function (f) {
      // UUID, distance, position, flags, owner's UUID, owner's name, name, description.
      var id = f[0], distance = f[1], position = f[2] || '', flags = f[3] || '-';
      var owner = unknown(f[5]) ? 'owner unknown' : 'of ' + f[5];
      var name = unknown(f[6]) ? '(name unknown)' : f[6];
      var description = unknown(f[7]) ? '' : f[7];
      var row = el('li');
      var who = el('div', 'who');
      who.appendChild(el('div', 'title', name));
      if (description !== '') { who.appendChild(el('div', null, description)); }
      who.appendChild(el('div', 'more', distance + ' m · at ' + position + ' · ' + owner + (flags === '-' ? '' : ' · ' + flags.split(',').join(', '))));
      row.appendChild(who);
      var acts = el('div', 'acts');
      var to = teleportTo(position);
      if (to) {
        acts.appendChild(rowButton('Teleport', function (button) {
          act(button, 'obj-tp:' + id, 'Teleport to ' + name, to, false);
        }));
      }
      acts.appendChild(rowButton('Sit', function (button) {
        act(button, 'obj-sit:' + id, 'Sit on ' + name, ['sit', id], false);
      }));
      acts.appendChild(rowButton('Touch', function (button) {
        act(button, 'obj-touch:' + id, 'Touch ' + name, ['touch', id], false);
      }));
      row.appendChild(acts);
      list.appendChild(row);
    });
  }

  // ------------------------------------------------------------ local chat

  // Who speaks, as the XMPP local chat contact names it.
  function speaker(rec) {
    if (rec.objectName !== '' || rec.objectId !== '') {
      return rec.objectName.trim() === '' ? '(object)' : rec.objectName.trim();
    }
    var legacy = rec.avatarName.trim(), display = rec.displayName.trim();
    if (legacy === '' && display === '') { return rec.avatarId || '(someone)'; }
    if (display === '') { return legacy; }
    if (legacy === '') { return display; }
    return sameName(legacy, display) ? legacy : display + ' (' + legacy + ')';
  }

  // A line of local chat, put together as that contact puts it together.
  function chatLine(rec) {
    var name = speaker(rec);
    var text = rec.text;
    if (text.slice(0, 3) === '/me' && (text[3] === ' ' || text[3] === '\'')) {
      return { text: name + text.slice(3), emote: true };
    }
    var how = rec.kind === 'say' ? '' : rec.kind === 'whisper' ? ' whispers' : rec.kind === 'shout' ? ' shouts' : ' [' + rec.kind + ']';
    return { text: name + how + ': ' + text, emote: false, other: how !== '' && how[1] === '[' };
  }

  function atEnd(log) {
    return log.scrollTop + log.clientHeight >= log.scrollHeight - 8;
  }

  function scrollToEnd(log) {
    log.scrollTop = log.scrollHeight;
  }

  function chatNode(rec) {
    var line = chatLine(rec);
    var node = el('div', 'line' + (line.emote ? ' emote' : '') + (line.other ? ' other' : ''));
    node.appendChild(timeNode(rec.time));
    node.appendChild(document.createTextNode(line.text));
    return node;
  }

  function addChat(rec) {
    var log = $('chat-log');
    state.chat.push(rec);
    log.appendChild(chatNode(rec));
    while (state.chat.length > MAX_CHAT_LINES) {
      state.chat.shift();
      if (log.firstChild) { log.removeChild(log.firstChild); }
    }
    if (state.tab === 'chat') {
      if (state.chatFollow) { scrollToEnd(log); }
      else if (!replaying) { show($('chat-new'), true); }
    } else if (!replaying) {
      show($('mark-chat'), true);
    }
  }

  function renderChat() {
    var log = $('chat-log');
    clear(log);
    state.chat.forEach(function (rec) { log.appendChild(chatNode(rec)); });
    show($('chat-new'), false);
  }

  function parseAvatars(text) {
    return listing(text).map(function (f) {
      return { id: f[0], distance: f[1], label: avatarLabel(f[3], f[4]) || (unknown(f[0]) ? 'Someone (hidden)' : f[0]) };
    });
  }

  function renderNearby(note) {
    var list = $('nearby-list');
    clear(list);
    (state.nearby || []).forEach(function (avatar) {
      var chip = el('li');
      chip.appendChild(document.createTextNode(avatar.label + ' '));
      chip.appendChild(el('span', 'more', unknown(avatar.distance) ? '' : avatar.distance + ' m'));
      list.appendChild(chip);
    });
    var text = note || (state.nearby && state.nearby.length === 0 ? 'Nobody within chat range.' : '');
    $('nearby-note').textContent = text;
    show($('nearby-note'), text !== '');
  }

  function nearbyAnswer(result) {
    if (result.ok) {
      state.nearby = parseAvatars(result.text);
      renderNearby('');
    } else {
      state.nearby = [];
      renderNearby(result.text);
    }
  }

  // The avatars within chat range. The page's own refreshes are marked as
  // such: they are not the operator's commands.
  function refreshNearby(own) {
    if (state.nearbyBusy || !$('login').hidden) { return; }
    state.nearbyBusy = true;
    run(['scan', 'avatars', 'chat'], own).then(function (result) {
      state.nearbyBusy = false;
      nearbyAnswer(result);
    });
  }

  function wireChat() {
    var log = $('chat-log');
    log.addEventListener('scroll', function () {
      if (state.tab !== 'chat') { return; }
      state.chatFollow = atEnd(log);
      if (state.chatFollow) { show($('chat-new'), false); }
    });
    $('chat-new').addEventListener('click', function () {
      state.chatFollow = true;
      scrollToEnd(log);
      show($('chat-new'), false);
    });
    $('chat-form').addEventListener('submit', function (event) {
      event.preventDefault();
      var input = $('chat-text');
      var text = input.value;
      if (text.trim() === '') { return; }
      act($('chat-button'), 'say', 'Say', ['say', text], true).then(function (result) {
        // The text stays in the box when it was not said.
        if (result && result.ok && input.value === text) { input.value = ''; }
        input.focus();
      });
    });
    $('nearby-refresh').addEventListener('click', function () {
      var button = $('nearby-refresh');
      if (button.disabled || state.nearbyBusy) { return; }
      state.nearbyBusy = true;
      button.disabled = true;
      button.classList.add('busy');
      // The operator asked: an ordinary command.
      run(['scan', 'avatars', 'chat'], false).then(function (result) {
        state.nearbyBusy = false;
        button.disabled = false;
        button.classList.remove('busy');
        nearbyAnswer(result);
      });
    });
  }

  // -------------------------------------------------------------------- IM

  // The conversation a message belongs to: an avatar's, an object's, or the
  // grid's for what carries neither's UUID.
  function conversationOf(rec) {
    if (rec.kind === 'object') {
      return rec.objectId ? { key: 'o:' + rec.objectId, kind: 'object', id: rec.objectId } : { key: 'grid', kind: 'grid', id: '' };
    }
    return rec.avatarId ? { key: 'a:' + rec.avatarId, kind: 'avatar', id: rec.avatarId } : { key: 'grid', kind: 'grid', id: '' };
  }

  function ensureConversation(key, kind, id) {
    var conv = state.convs[key];
    if (!conv) {
      conv = { key: key, kind: kind, id: id, name: '', display: '', objectName: '', messages: [], unread: 0, draft: '', last: 0, opened: 0 };
      state.convs[key] = conv;
    }
    return conv;
  }

  function conversationTitle(conv) {
    if (conv.kind === 'grid') { return 'The grid'; }
    if (conv.kind === 'object') { return conv.objectName || '(object)'; }
    if (conv.kind === 'name') { return conv.name; }
    return avatarLabel(conv.name, conv.display) || conv.id;
  }

  function sentKind(kind) { return kind === 'im_out' || kind === 'im_auto'; }

  function addIm(rec) {
    var where = conversationOf(rec);
    var conv = ensureConversation(where.key, where.kind, where.id);
    if (conv.kind === 'avatar') {
      if (rec.avatarName) { conv.name = rec.avatarName; }
      if (rec.displayName) { conv.display = rec.displayName; }
    } else if (conv.kind === 'object' && rec.objectName) {
      conv.objectName = rec.objectName;
    }
    conv.messages.push(rec);
    conv.last = Math.max(conv.last, rec.time);
    var looking = state.tab === 'im' && state.conv === conv.key && document.visibilityState === 'visible';
    if (!sentKind(rec.kind) && !replaying && !looking) { conv.unread++; }
    // A first message sent by name, whose answer did not say whom it went
    // to: this is where it went.
    var pending = state.pendingSent;
    if (pending && rec.kind === 'im_out' && conv.kind === 'avatar' && rec.text === pending.text) {
      state.pendingSent = null;
      merge(pending.key, conv.key);
    }
    renderConversations();
    if (state.conv === conv.key) { appendMessage(rec, true); }
  }

  // The messages are gone: the bot sends what it kept anew. Conversations
  // the operator opened stay, empty.
  function dropMessages() {
    // What is being typed stays where it is.
    if (state.conv && state.convs[state.conv]) { state.convs[state.conv].draft = $('im-text').value; }
    state.chat = [];
    renderChat();
    Object.keys(state.convs).forEach(function (key) {
      var conv = state.convs[key];
      conv.messages = [];
      conv.unread = 0;
      conv.last = 0;
      if (!conv.opened && state.conv !== key) { delete state.convs[key]; }
    });
    renderConversations();
    renderConversation();
  }

  function seen(key) {
    var conv = key ? state.convs[key] : null;
    if (conv && state.tab === 'im') { conv.unread = 0; }
  }

  function renderConversations() {
    var list = $('im-conversations');
    clear(list);
    var convs = Object.keys(state.convs).map(function (key) { return state.convs[key]; });
    convs.sort(function (a, b) { return (b.last || b.opened) - (a.last || a.opened); });
    var unread = 0;
    convs.forEach(function (conv) {
      unread += conv.unread;
      var item = el('li', conv.key === state.conv ? 'current' : null);
      item.appendChild(el('span', 'label', conversationTitle(conv)));
      if (conv.kind === 'object') { item.appendChild(el('span', 'more', 'object')); }
      if (conv.unread > 0) { item.appendChild(el('span', 'count', String(conv.unread))); }
      item.addEventListener('click', function () { choose(conv.key); });
      list.appendChild(item);
    });
    if (convs.length === 0) { list.appendChild(el('li', 'more', 'None yet.')); }
    show($('mark-im'), unread > 0);
  }

  function messageNode(rec) {
    var sent = sentKind(rec.kind);
    var node = el('div', 'msg' + (sent ? ' sent' : ''));
    var bubble = el('div', 'bubble');
    var tags = [timeLabel(rec.time)];
    if (rec.kind === 'im_auto') { tags.push('sent by the bot itself'); }
    if (rec.offline) { tags.push('stored while offline'); }
    var tag = el('span', 'tag', tags.join(' · '));
    tag.title = new Date(rec.time * 1000).toLocaleString();
    bubble.appendChild(tag);
    bubble.appendChild(document.createTextNode(rec.text));
    node.appendChild(bubble);
    return node;
  }

  function appendMessage(rec, follow) {
    var log = $('im-log');
    var end = atEnd(log);
    log.appendChild(messageNode(rec));
    if (follow && (end || sentKind(rec.kind))) { scrollToEnd(log); }
  }

  function renderConversation() {
    var conv = state.conv ? state.convs[state.conv] : null;
    var log = $('im-log');
    clear(log);
    $('im-title').textContent = conv ? conversationTitle(conv) : 'No conversation chosen';
    var canAnswer = !!conv && (conv.kind === 'avatar' || conv.kind === 'name');
    show($('im-form'), canAnswer);
    show($('im-readonly'), !!conv && !canAnswer);
    show($('im-close'), !!conv && conv.messages.length === 0);
    if (!conv) { return; }
    conv.messages.forEach(function (rec) { log.appendChild(messageNode(rec)); });
    $('im-text').value = conv.draft;
    scrollToEnd(log);
  }

  function choose(key) {
    var before = state.conv ? state.convs[state.conv] : null;
    if (before) { before.draft = $('im-text').value; }
    state.conv = key;
    seen(key);
    renderConversations();
    renderConversation();
    if (key && state.tab === 'im') { $('im-text').focus(); }
  }

  // Moves what the operator typed in one conversation into another, which
  // is the avatar's the first turned out to be.
  function merge(fromKey, toKey) {
    var from = state.convs[fromKey];
    if (!from || fromKey === toKey) { return; }
    var to = state.convs[toKey];
    if (!to) { return; }
    to.opened = to.opened || from.opened;
    if (state.conv === fromKey) {
      to.draft = $('im-text').value;
      state.conv = toKey;
    }
    delete state.convs[fromKey];
    seen(toKey);
    renderConversations();
    renderConversation();
  }

  function openAvatar(id, name, display) {
    id = id.toLowerCase();
    var conv = ensureConversation('a:' + id, 'avatar', id);
    if (name && !conv.name) { conv.name = name; }
    if (display && !conv.display) { conv.display = display; }
    conv.opened = conv.opened || Date.now() / 1000;
    choose(conv.key);
  }

  // A name as the bot compares names: `first.last` and one word alike.
  function plainName(name) {
    var words = name.trim().toLowerCase().replace(/\./g, ' ').split(/\s+/);
    if (words.length === 1) { words.push('resident'); }
    return words.join(' ');
  }

  function openByName(typed) {
    typed = typed.trim();
    if (typed === '') { return; }
    if (UUID.test(typed)) { openAvatar(typed, '', ''); return; }
    // Someone the page already knows by that name.
    var wanted = plainName(typed);
    var known = Object.keys(state.convs).filter(function (key) {
      var conv = state.convs[key];
      return conv.kind === 'avatar' && conv.name !== '' && plainName(conv.name) === wanted;
    });
    if (known.length === 1) { choose(known[0]); return; }
    var conv = ensureConversation('n:' + wanted, 'name', '');
    conv.name = typed;
    conv.opened = conv.opened || Date.now() / 1000;
    choose(conv.key);
  }

  function sendIm() {
    var conv = state.conv ? state.convs[state.conv] : null;
    var input = $('im-text');
    var text = input.value;
    if (!conv || text.trim() === '') { return; }
    var byName = conv.kind === 'name';
    var key = conv.key;
    act($('im-button'), 'im', 'Message to ' + conversationTitle(conv), ['im', byName ? conv.name : conv.id, text], true).then(function (result) {
      if (!result || !result.ok) { input.focus(); return; }
      if (state.conv === key && input.value === text) { input.value = ''; }
      if (state.convs[key]) { state.convs[key].draft = ''; }
      if (byName) {
        // The answer names whom it went to: `sent to Jane Doe <uuid>`.
        var found = /^sent to (.*?)\s*<([0-9a-fA-F-]{36})>\s*$/.exec(result.text) || /^sent to()\s*([0-9a-fA-F-]{36})\s*$/.exec(result.text);
        if (found && UUID.test(found[2])) {
          var id = found[2].toLowerCase();
          var to = ensureConversation('a:' + id, 'avatar', id);
          if (found[1] && !to.name) { to.name = found[1]; }
          merge(key, to.key);
        } else {
          state.pendingSent = { key: key, text: text };
        }
      }
      input.focus();
    });
  }

  function renderFriends(note) {
    var list = $('im-friends');
    clear(list);
    (state.friends || []).forEach(function (friend) {
      var item = el('li');
      var dot = el('span', 'dot' + (friend.status === 'online' ? ' online' : friend.status === 'offline' ? '' : ' unknown'));
      item.appendChild(dot);
      item.appendChild(el('span', 'label', friend.label));
      item.appendChild(el('span', 'more', friend.status === '?' ? 'not shown' : friend.status));
      item.addEventListener('click', function () { openAvatar(friend.id, friend.name, friend.display); });
      list.appendChild(item);
    });
    var text = note || (state.friends && state.friends.length === 0 ? 'No friends.' : '');
    $('friends-note').textContent = text;
    show($('friends-note'), text !== '');
  }

  function friendsAnswer(result) {
    if (result.ok) {
      // UUID, status, name, display name; in the bot's order, online first.
      state.friends = listing(result.text).map(function (f) {
        var name = unknown(f[2]) ? '' : f[2], display = unknown(f[3]) ? '' : f[3];
        return { id: f[0], status: f[1], name: name, display: display, label: avatarLabel(name, display) || f[0] };
      });
      renderFriends('');
    } else {
      state.friends = [];
      renderFriends(result.text);
    }
  }

  function refreshFriends(own) {
    if (state.friendsBusy || !$('login').hidden) { return; }
    state.friendsBusy = true;
    run(['friends'], own).then(function (result) {
      state.friendsBusy = false;
      friendsAnswer(result);
    });
  }

  function wireIm() {
    $('im-open-form').addEventListener('submit', function (event) {
      event.preventDefault();
      var input = $('im-open-name');
      openByName(input.value);
      input.value = '';
    });
    $('im-form').addEventListener('submit', function (event) {
      event.preventDefault();
      sendIm();
    });
    $('im-close').addEventListener('click', function () {
      var key = state.conv;
      if (key && state.convs[key] && state.convs[key].messages.length === 0) { delete state.convs[key]; }
      state.conv = null;
      $('im-text').value = '';
      renderConversations();
      renderConversation();
    });
    $('friends-refresh').addEventListener('click', function () {
      var button = $('friends-refresh');
      if (button.disabled || state.friendsBusy) { return; }
      state.friendsBusy = true;
      button.disabled = true;
      button.classList.add('busy');
      run(['friends'], false).then(function (result) {
        state.friendsBusy = false;
        button.disabled = false;
        button.classList.remove('busy');
        friendsAnswer(result);
      });
    });
    // Coming back to the window is looking at the conversation that is shown.
    document.addEventListener('visibilitychange', function () {
      if (document.visibilityState === 'visible' && state.tab === 'im' && state.conv) {
        seen(state.conv);
        renderConversations();
      }
    });
  }

  // ------------------------------------------------------------- inventory

  // UUID, type, attach or wear point, `attached` or `-`, path.
  function inventoryEntries(text, parent) {
    var entries = listing(text).map(function (f) {
      var names = pathNames(f[4] || '');
      return {
        id: f[0],
        type: f[1] || 'other',
        point: f[2] && f[2] !== '-' ? f[2] : '',
        attached: f[3] === 'attached',
        path: names.join('/'),
        name: names.length ? names[names.length - 1] : '(unnamed)',
        parent: parent,
        open: false,
        children: null,
        loading: false,
        error: ''
      };
    });
    // Folders first, each kind in the order of the answer.
    var folders = entries.filter(function (entry) { return entry.type === 'folder'; });
    var items = entries.filter(function (entry) { return entry.type !== 'folder'; });
    return folders.concat(items);
  }

  // Lists a folder, or My Inventory itself for `null`: its direct entries
  // only. Folders that were open stay open, and are listed again as well.
  function loadFolder(node) {
    var inventory = state.inventory;
    var old = node ? node.children : inventory.root;
    var args = ['inventory', '-n'];
    if (node) { args.push(node.id); node.loading = true; }
    renderInventory();
    return run(args, false).then(function (result) {
      if (node) { node.loading = false; }
      if (!result.ok) {
        if (node) { node.error = result.text; } else { inventory.root = null; inventory.rootError = result.text; }
        renderInventory();
        return;
      }
      var entries = inventoryEntries(result.text, node);
      var reopen = [];
      (old || []).forEach(function (was) {
        if (!was.open) { return; }
        entries.forEach(function (entry) {
          if (entry.id === was.id && entry.type === 'folder') {
            entry.open = true;
            entry.children = was.children;
            reopen.push(entry);
          }
        });
      });
      if (node) { node.children = entries; node.error = ''; } else { inventory.root = entries; inventory.rootError = ''; }
      renderInventory();
      reopen.forEach(loadFolder);
    });
  }

  function loadAttached() {
    var inventory = state.inventory;
    return run(['inventory', '-a'], false).then(function (result) {
      if (result.ok) {
        inventory.attached = inventoryEntries(result.text, null);
        inventory.attachedError = '';
      } else {
        inventory.attached = null;
        inventory.attachedError = result.text;
      }
      renderInventory();
    });
  }

  function reloadInventory() {
    return state.inventory.attachedOnly ? loadAttached() : loadFolder(null);
  }

  // What an entry can be asked to do: by its type and whether it is attached.
  function inventoryActions(entry) {
    if (entry.type === 'object') {
      return entry.attached ? [['Detach', 'detach'], ['Touch', 'touch']] : [['Attach', 'attach']];
    }
    if (entry.type === 'clothing' || entry.type === 'link') { return [['Wear', 'wear'], ['Take off', 'detach']]; }
    if (entry.type === 'bodypart') { return [['Wear', 'wear']]; }
    return [];
  }

  function entryNode(entry, withPath) {
    var item = el('li');
    var row = el('div', 'entry' + (entry.type === 'folder' ? ' folder' : ''));
    if (entry.type === 'folder') {
      row.appendChild(el('span', 'twist', entry.open ? '▾' : '▸'));
      row.appendChild(el('span', 'name', entry.name));
      row.setAttribute('role', 'button');
      row.tabIndex = 0;
      var toggle = function () {
        entry.open = !entry.open;
        if (entry.open && entry.children === null) { loadFolder(entry); } else { renderInventory(); }
      };
      row.addEventListener('click', function (event) {
        if (event.target.tagName !== 'BUTTON') { toggle(); }
      });
      row.addEventListener('keydown', function (event) {
        if ((event.key === 'Enter' || event.key === ' ') && event.target === row) { event.preventDefault(); toggle(); }
      });
      if (entry.open) {
        var again = rowButton('List again', function () { loadFolder(entry); });
        var acts = el('span', 'acts');
        acts.appendChild(again);
        row.appendChild(acts);
      }
    } else {
      row.appendChild(el('span', 'twist', ''));
      row.appendChild(el('span', 'name', withPath ? entry.path : entry.name));
      row.appendChild(el('span', 'more', entry.type + (entry.point ? ' · ' + entry.point : '')));
      if (entry.attached) { row.appendChild(el('span', 'attached', 'attached')); }
      var buttons = el('span', 'acts');
      inventoryActions(entry).forEach(function (action) {
        buttons.appendChild(rowButton(action[0], function (button) {
          var key = 'inv:' + entry.id;
          act(button, key, action[0] + ' ' + entry.name, [action[1], entry.id], false).then(function (result) {
            if (!result || !result.ok) { return; }
            // The entry shows what is attached now.
            if (state.inventory.attachedOnly) { loadAttached(); } else { loadFolder(entry.parent); }
          });
        }));
      });
      row.appendChild(buttons);
    }
    item.appendChild(row);
    if (entry.type === 'folder' && entry.open) {
      var inside = el('ul');
      if (entry.error) { inside.appendChild(el('li', 'error', entry.error)); }
      else if (entry.children === null || (entry.loading && entry.children.length === 0)) { inside.appendChild(el('li', 'more', 'Reading…')); }
      else if (entry.children.length === 0) { inside.appendChild(el('li', 'more', 'Empty.')); }
      else { entry.children.forEach(function (child) { inside.appendChild(entryNode(child, false)); }); }
      item.appendChild(inside);
    }
    return item;
  }

  function renderInventory() {
    var inventory = state.inventory;
    var tree = $('inventory-tree');
    var note = $('inventory-note');
    clear(tree);
    $('inventory-attached').checked = inventory.attachedOnly;
    var entries = inventory.attachedOnly ? inventory.attached : inventory.root;
    var error = inventory.attachedOnly ? inventory.attachedError : inventory.rootError;
    var text = '';
    if (error) { text = error; }
    else if (!inventory.started) { text = ''; }
    else if (!entries) { text = 'Reading…'; }
    else if (entries.length === 0) { text = inventory.attachedOnly ? 'Nothing is attached.' : 'The inventory is empty.'; }
    note.textContent = text;
    note.classList.toggle('error', !!error);
    show(note, text !== '');
    if (entries && !error) {
      entries.forEach(function (entry) { tree.appendChild(entryNode(entry, inventory.attachedOnly)); });
    }
  }

  function wireInventory() {
    $('inventory-attached').addEventListener('change', function () {
      state.inventory.attachedOnly = $('inventory-attached').checked;
      renderInventory();
      reloadInventory();
    });
    $('inventory-refresh').addEventListener('click', function () {
      var button = $('inventory-refresh');
      if (button.disabled) { return; }
      button.disabled = true;
      button.classList.add('busy');
      reloadInventory().then(function () {
        button.disabled = false;
        button.classList.remove('busy');
      });
    });
  }

  // --------------------------------------------------------------- console

  function wireConsole() {
    var input = $('console-line');
    var out = $('console-out');
    $('console-form').addEventListener('submit', function (event) {
      event.preventDefault();
      var line = input.value;
      if (line.trim() === '') { return; }
      state.history.push(line);
      state.historyAt = -1;
      input.value = '';
      out.appendChild(el('div', 'sent', line));
      // The answer has its place under its line, whenever it comes.
      var answer = el('div', 'answer wait', '…');
      out.appendChild(answer);
      scrollToEnd(out);
      runLine(line).then(function (result) {
        var end = atEnd(out);
        answer.className = 'answer' + (result.ok ? '' : ' error');
        answer.textContent = result.ok ? (result.text || 'ok') : 'error: ' + result.text;
        if (end) { scrollToEnd(out); }
      });
    });
    input.addEventListener('keydown', function (event) {
      var history = state.history;
      if (event.key === 'ArrowUp' && history.length > 0) {
        event.preventDefault();
        state.historyAt = state.historyAt < 0 ? history.length - 1 : Math.max(0, state.historyAt - 1);
        input.value = history[state.historyAt];
      } else if (event.key === 'ArrowDown' && state.historyAt >= 0) {
        event.preventDefault();
        state.historyAt++;
        if (state.historyAt >= history.length) { state.historyAt = -1; input.value = ''; }
        else { input.value = history[state.historyAt]; }
      }
    });
  }

  // -------------------------------------------------------------- settings

  function wireSettings() {
    var stored = '';
    try { stored = window.localStorage.getItem('theme') || ''; } catch (e) { /* no storage */ }
    if (stored !== 'light' && stored !== 'dark') { stored = ''; }
    var choices = $('theme-choices').querySelectorAll('input[name="theme"]');
    for (var i = 0; i < choices.length; i++) {
      choices[i].checked = choices[i].value === stored;
      choices[i].addEventListener('change', function (event) {
        var theme = event.target.value;
        if (theme === '') { document.documentElement.removeAttribute('data-theme'); }
        else { document.documentElement.setAttribute('data-theme', theme); }
        try {
          if (theme === '') { window.localStorage.removeItem('theme'); }
          else { window.localStorage.setItem('theme', theme); }
        } catch (e) { /* the choice lasts as long as the page */ }
      });
    }
  }

  // ----------------------------------------------------------------- start

  function renderEverything() {
    renderStatus();
    renderChat();
    renderNearby('');
    renderConversations();
    renderConversation();
    renderFriends('');
    renderInventory();
    clear($('avatars-list'));
    clear($('objects-list'));
    show($('avatars-made'), false);
    show($('objects-made'), false);
    clear($('console-out'));
    show($('mark-chat'), false);
    var inputs = document.querySelectorAll('#app input:not([type="radio"]):not([type="checkbox"])');
    for (var i = 0; i < inputs.length; i++) { inputs[i].value = ''; inputs[i].classList.remove('bad'); }
  }

  function start() {
    state = freshState();
    wireLogin();
    wireWorld();
    wireChat();
    wireIm();
    wireInventory();
    wireConsole();
    wireSettings();
    renderEverything();
    window.addEventListener('hashchange', route);
    // The lists the page keeps up to date, while their tab is looked at.
    window.setInterval(function () {
      if (state.tab === 'chat' && document.visibilityState === 'visible' && !$('app').hidden && !state.lost) { refreshNearby(true); }
    }, NEARBY_EVERY);
    window.setInterval(function () {
      if (state.tab === 'im' && document.visibilityState === 'visible' && !$('app').hidden && !state.lost) { refreshFriends(true); }
    }, FRIENDS_EVERY);
    begin();
  }

  if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', start); }
  else { start(); }
})();
