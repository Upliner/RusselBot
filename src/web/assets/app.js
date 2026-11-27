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
  var TABS = ['world', 'chat', 'im', 'groups', 'inventory', 'money', 'console', 'settings'];
  var NEARBY_EVERY = 15000;
  var FRIENDS_EVERY = 60000;
  var MAX_CHAT_LINES = 1000;
  var MAX_TRANSACTIONS = 1000;
  var OTHER_AMOUNT = 'other';
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
      groups: freshGroups(),  // the Groups tab
      inventory: { root: null, attachedOnly: false, attached: null, started: false },
      history: [],            // console lines sent
      historyAt: -1,
      money: [],              // transactions, oldest first
      payees: null,           // the avatars of the answer of `payees`
      payeesError: '',
      payeesBusy: false,
      payeesAsked: false,     // the Money tab was shown: `payees` was sent
      payee: null,            // the payee chosen from a list: {shown, pay, id}
      paying: false,          // the Send money form's payment is under way
      focusAmount: false      // the amount box gets the cursor when the tab is shown
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
  var freshStream = false;  // no status has come over the stream yet

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
    freshStream = true;
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
    source.addEventListener('group', function (event) {
      if (event.lastEventId) { lastId = event.lastEventId; }
      addGroupMessage(groupRecord(fields(event)));
    });
    source.addEventListener('groups', function () {
      // The groups or the invitations changed: not kept, not replayed.
      refreshGroups(true);
    });
    source.addEventListener('money', function (event) {
      if (event.lastEventId) { lastId = event.lastEventId; }
      addTransaction(transaction(fields(event)));
    });
    source.addEventListener('status', function (event) {
      // The status ends what a page is sent when it opens its stream.
      replaying = false;
      var first = freshStream;
      freshStream = false;
      var wasLost = state.lost;
      setStatus(unescapeField(event.data));
      found();
      // The bot is there now: what waits may have changed meanwhile. Once
      // found again, `refreshOwn` asks already.
      if (first && !wasLost && !groupsState().busy) { refreshGroups(true); }
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
    if (state.tab === 'im' || state.tab === 'money') { refreshFriends(true); }
    if (state.payeesAsked && canPay()) { refreshPayees(true); }
    refreshGroups(true);
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
    closeDialog();
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
    if (name === 'money' && !canPay()) { return 'world'; }
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
    } else if (tab === 'groups') {
      if (!state.groups.asked) { state.groups.asked = true; refreshGroups(true); }
      groupSeen();
      renderGroupList();
      if (state.groups.follow) { scrollToEnd($('group-log')); }
    } else if (tab === 'inventory') {
      if (!state.inventory.started) { state.inventory.started = true; reloadInventory(); }
    } else if (tab === 'money') {
      refreshFriends(true);
      if (!state.payeesAsked) { state.payeesAsked = true; refreshPayees(true); }
      if (state.focusAmount) { state.focusAmount = false; $('pay-amount').focus(); }
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
    var could = canPay();
    var wasConnected = connected();
    state.status = status;
    renderStatus();
    if (canPay() !== could) { renderPayControls(); }
    if (connected() !== wasConnected) { renderGroupList(); renderInvitations(); }
    renderDescription();
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
        continue;
      }
      panel.appendChild(statusItem('Avatar', status.avatar || '?'));
      if (status.connected !== 'yes') {
        panel.appendChild(el('span', 'item off', 'not connected'));
        if (status.mode) { panel.appendChild(statusItem('Mode', status.mode)); }
      } else {
        panel.appendChild(statusItem('Region', status.region || '?'));
        panel.appendChild(statusItem('Position', status.position || '?'));
        panel.appendChild(statusItem('Seat', seatText(status)));
        panel.appendChild(statusItem('Mode', status.mode || '?'));
      }
    }
    // The balance and the limit are the Money tab's.
    renderMoneyHeader();
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
      // An object that takes money: what it offers is asked for first.
      var area = null;
      if (flags.split(',').indexOf('pay') >= 0) {
        area = el('div', 'pay-area pay-control');
        var pay = rowButton('Pay', function (button) { askPrices(button, area, id, name); });
        pay.classList.add('pay-control');
        show(pay, canPay());
        show(area, canPay());
        acts.appendChild(pay);
      }
      row.appendChild(acts);
      if (area) { row.appendChild(area); }
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
    state.money = [];
    renderTransactions();
    Object.keys(state.convs).forEach(function (key) {
      var conv = state.convs[key];
      conv.messages = [];
      conv.unread = 0;
      conv.last = 0;
      if (!conv.opened && state.conv !== key) { delete state.convs[key]; }
    });
    renderConversations();
    renderConversation();
    var groups = groupsState();
    saveGroupDrafts();
    Object.keys(groups.known).forEach(function (id) {
      var group = groups.known[id];
      group.messages = [];
      group.unread = 0;
      group.last = 0;
    });
    renderGroupList();
    renderGroupView();
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
    renderImPay();
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

  function friendNode(friend, onClick) {
    var item = el('li');
    var dot = el('span', 'dot' + (friend.status === 'online' ? ' online' : friend.status === 'offline' ? '' : ' unknown'));
    item.appendChild(dot);
    item.appendChild(el('span', 'label', friend.label));
    item.appendChild(el('span', 'more', friend.status === '?' ? 'not shown' : friend.status));
    item.addEventListener('click', onClick);
    return item;
  }

  // The friends, on the IM tab and as payees on the Money tab.
  function renderFriends(note) {
    var list = $('im-friends');
    var payees = $('money-friends');
    clear(list);
    clear(payees);
    (state.friends || []).forEach(function (friend) {
      list.appendChild(friendNode(friend, function () { openAvatar(friend.id, friend.name, friend.display); }));
      payees.appendChild(friendNode(friend, function () { choosePayee(friend.label, friend.id, friend.id); }));
    });
    var none = state.friends && state.friends.length === 0 ? 'No friends.' : '';
    var text = note || none;
    $('friends-note').textContent = text;
    show($('friends-note'), text !== '');
    var moneyText = note ? 'The friend list could not be read: ' + note : none;
    var moneyNote = $('money-friends-note');
    moneyNote.textContent = moneyText;
    moneyNote.classList.toggle('error', !!note);
    show(moneyNote, moneyText !== '');
    renderInviteFriends();
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
    // Pay in an avatar's conversation: the Money tab, with that avatar as
    // the payee by its UUID and nothing sent.
    $('im-pay').addEventListener('click', function () {
      var conv = state.conv ? state.convs[state.conv] : null;
      if (!conv || conv.kind !== 'avatar' || !canPay()) { return; }
      choosePayee(conversationTitle(conv), conv.id, conv.id);
      var amount = $('pay-amount');
      amount.value = '';
      amount.classList.remove('bad');
      state.focusAmount = true;
      window.location.hash = '#money';
    });
  }

  // ---------------------------------------------------------------- groups

  function freshGroups() {
    return {
      asked: false,           // the tab was shown: the lists were asked for
      list: null,             // the answer of `group list`: [{id, name, flags}]
      listError: '',
      invites: null,          // the answer of `group invitations`
      invitesError: '',
      busy: false,            // the two are being asked for
      again: false,           // and are to be asked for once more after
      known: {},              // by UUID: the groups the page has anything of
      chosen: null,           // the UUID of the group shown
      follow: true,           // the group's view is at its end
      answering: {}           // invitations being answered, by group UUID: {text, kind}
    };
  }

  function groupsState() {
    if (!state.groups) { state.groups = freshGroups(); }
    return state.groups;
  }

  function ensureGroup(id) {
    var groups = groupsState();
    var group = groups.known[id];
    if (!group) {
      group = {
        id: id, name: '', messages: [], unread: 0, last: 0, draft: '',
        info: null,           // {busy, error, lines}
        invite: null          // the Invite form: {text, chosen, result}
      };
      groups.known[id] = group;
    }
    return group;
  }

  function listedGroup(id) {
    var list = groupsState().list;
    if (!list) { return null; }
    for (var i = 0; i < list.length; i++) { if (list[i].id === id) { return list[i]; } }
    return null;
  }

  function groupName(id) {
    var listed = listedGroup(id);
    if (listed && listed.name) { return listed.name; }
    var group = groupsState().known[id];
    return group && group.name ? group.name : id;
  }

  function connected() {
    return !!state.status && state.status.connected === 'yes';
  }

  // A group message as the event stream gives it.
  function groupRecord(f) {
    return {
      time: parseInt(f[0], 10) || 0,
      kind: f[1] || '',
      groupId: (f[2] || '').toLowerCase(),
      groupName: f[3] || '',
      avatarId: (f[4] || '').toLowerCase(),
      avatarName: f[5] || '',
      displayName: f[6] || '',
      subject: f[7] || '',
      attachment: f[8] || '',
      offline: f[9] === '1',
      text: f[10] || ''
    };
  }

  function lookingAtGroup(id) {
    return state.tab === 'groups' && groupsState().chosen === id && document.visibilityState === 'visible';
  }

  function addGroupMessage(rec) {
    if (!rec.groupId) { return; }
    var group = ensureGroup(rec.groupId);
    if (rec.groupName && !unknown(rec.groupName)) { group.name = rec.groupName; }
    group.messages.push(rec);
    while (group.messages.length > MAX_CHAT_LINES) { group.messages.shift(); }
    group.last = Math.max(group.last, rec.time);
    if (rec.kind !== 'chat_out' && !replaying && !lookingAtGroup(group.id)) { group.unread++; }
    renderGroupList();
    if (groupsState().chosen !== group.id) { return; }
    var log = $('group-log');
    log.appendChild(groupMessageNode(rec));
    while (log.childNodes.length > MAX_CHAT_LINES) { log.removeChild(log.firstChild); }
    if (state.tab !== 'groups') { return; }
    if (groupsState().follow || rec.kind === 'chat_out') { scrollToEnd(log); }
    else if (!replaying) { show($('group-new'), true); }
  }

  // Who wrote a group message: as the Local chat tab names a speaker.
  function groupSpeaker(rec) {
    return speaker({ objectName: '', objectId: '', avatarId: rec.avatarId, avatarName: rec.avatarName, displayName: rec.displayName });
  }

  function groupMessageNode(rec) {
    if (rec.kind === 'notice') {
      var node = el('div', 'gnotice');
      var head = el('div', 'head');
      head.appendChild(el('span', 'badge', 'Notice'));
      head.appendChild(timeNode(rec.time));
      head.appendChild(el('span', 'from', groupSpeaker(rec)));
      if (rec.offline) { head.appendChild(el('span', 'stored', 'stored while offline')); }
      node.appendChild(head);
      node.appendChild(el('div', 'subject', rec.subject || '(no subject)'));
      if (rec.text !== '') { node.appendChild(el('div', 'body', rec.text)); }
      if (rec.attachment !== '') {
        var attached = el('div', 'attachment');
        attached.appendChild(document.createTextNode('Attached: '));
        attached.appendChild(el('span', 'item', rec.attachment));
        attached.appendChild(document.createTextNode(' (cannot be taken from the page)'));
        node.appendChild(attached);
      }
      return node;
    }
    var own = rec.kind === 'chat_out';
    var line = el('div', 'line' + (own ? ' own' : '') + (rec.kind !== 'chat' && !own ? ' other' : ''));
    line.appendChild(timeNode(rec.time));
    if (own) {
      line.appendChild(el('span', 'you', 'you'));
      line.appendChild(document.createTextNode(rec.text));
    } else {
      var how = rec.kind === 'chat' ? '' : ' [' + rec.kind + ']';
      line.appendChild(document.createTextNode(groupSpeaker(rec) + how + ': ' + rec.text));
    }
    if (rec.offline) { line.appendChild(el('span', 'stored', 'stored while offline')); }
    return line;
  }

  // The groups the list shows: the avatar's in the order of the answer,
  // then those the page has messages of, newest first.
  function shownGroups() {
    var groups = groupsState();
    var out = [];
    var seenIds = {};
    (groups.list || []).forEach(function (listed) {
      seenIds[listed.id] = true;
      out.push({ id: listed.id, listed: listed });
    });
    var others = Object.keys(groups.known).filter(function (id) {
      return !seenIds[id] && (groups.known[id].messages.length > 0 || groups.chosen === id);
    }).map(function (id) { return groups.known[id]; });
    others.sort(function (a, b) { return b.last - a.last; });
    others.forEach(function (group) { out.push({ id: group.id, listed: null }); });
    return out;
  }

  function groupsMark() {
    var groups = groupsState();
    var unread = 0;
    Object.keys(groups.known).forEach(function (id) { unread += groups.known[id].unread; });
    var waiting = !!groups.invites && groups.invites.length > 0;
    var mark = $('mark-groups');
    mark.title = waiting && unread > 0 ? 'An invitation waits; new messages' : waiting ? 'An invitation waits' : 'New messages';
    show(mark, waiting || unread > 0);
  }

  function renderGroupList() {
    var groups = groupsState();
    var list = $('groups-list');
    clear(list);
    var note = '';
    var isError = false;
    if (!connected() && state.status) { note = 'The avatar is not connected.'; }
    else if (groups.listError) { note = groups.listError; isError = true; }
    else if (groups.list && groups.list.length === 0) { note = 'The avatar is in no group.'; }
    else if (!groups.list && groups.busy) { note = 'Asking for the groups…'; }
    var noteNode = $('groups-note');
    noteNode.textContent = note;
    noteNode.classList.toggle('error', isError);
    show(noteNode, note !== '');
    shownGroups().forEach(function (entry) {
      var group = groups.known[entry.id];
      var item = el('li', (entry.id === groups.chosen ? 'current' : '') + (entry.listed ? '' : ' gone'));
      item.appendChild(el('span', 'label', groupName(entry.id)));
      if (entry.listed && entry.listed.flags.indexOf('active') >= 0) { item.appendChild(el('span', 'active', 'active')); }
      if (!entry.listed && groups.list) { item.appendChild(el('span', 'more', 'not a member')); }
      if (group && group.unread > 0) { item.appendChild(el('span', 'count', String(group.unread))); }
      item.tabIndex = 0;
      item.setAttribute('role', 'button');
      item.addEventListener('click', function () { chooseGroup(entry.id); });
      item.addEventListener('keydown', function (event) {
        if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); chooseGroup(entry.id); }
      });
      list.appendChild(item);
    });
    groupsMark();
  }

  // The chosen group's messages were seen.
  function groupSeen() {
    var groups = groupsState();
    var group = groups.chosen ? groups.known[groups.chosen] : null;
    if (group && state.tab === 'groups' && document.visibilityState === 'visible') { group.unread = 0; }
  }

  function saveGroupDrafts() {
    var groups = groupsState();
    var group = groups.chosen ? groups.known[groups.chosen] : null;
    if (!group) { return; }
    group.draft = $('group-text').value;
    if (group.invite) { group.invite.text = $('group-invite-who').value; }
  }

  function chooseGroup(id) {
    var groups = groupsState();
    saveGroupDrafts();
    groups.chosen = id;
    ensureGroup(id);
    groups.follow = true;
    groupSeen();
    renderGroupList();
    renderGroupView();
    if (listedGroup(id) && state.tab === 'groups') { $('group-text').focus(); }
  }

  function renderGroupView() {
    var groups = groupsState();
    var id = groups.chosen;
    var group = id ? ensureGroup(id) : null;
    var listed = id ? listedGroup(id) : null;
    var flags = listed ? listed.flags : [];
    var log = $('group-log');
    clear(log);
    show($('group-new'), false);
    $('group-title').textContent = group ? groupName(id) : 'No group chosen';
    show($('group-active'), !!listed && flags.indexOf('active') >= 0);
    show($('group-activate'), !!listed && flags.indexOf('active') < 0);
    show($('group-invite'), !!listed && flags.indexOf('invite') >= 0);
    show($('group-info'), !!group);
    show($('group-leave'), !!listed);
    var gone = $('group-gone');
    gone.textContent = !group || listed ? '' : groups.list ? 'The avatar is not in this group.' : 'The avatar\'s groups are not known right now.';
    show(gone, gone.textContent !== '');
    show($('group-form'), !!listed);
    show(log, !!group);
    renderGroupInfo();
    renderGroupInvite();
    if (!group) { return; }
    group.messages.forEach(function (rec) { log.appendChild(groupMessageNode(rec)); });
    if (group.messages.length === 0) { log.appendChild(el('div', 'made', 'No messages yet.')); }
    $('group-text').value = group.draft;
    scrollToEnd(log);
  }

  // ------------------------------------------------ refreshing the lists

  function groupsListAnswer(result) {
    var groups = groupsState();
    if (result.ok) {
      // UUID, flags, name.
      groups.list = listing(result.text).map(function (f) {
        var id = (f[0] || '').toLowerCase();
        return { id: id, name: f[2] || '', flags: !f[1] || f[1] === '-' ? [] : f[1].split(',') };
      }).filter(function (listed) { return listed.id !== ''; });
      // The name stays known for a group the avatar leaves.
      groups.list.forEach(function (listed) {
        var group = ensureGroup(listed.id);
        if (listed.name) { group.name = listed.name; }
      });
      groups.listError = '';
    } else {
      groups.list = null;
      groups.listError = result.text;
    }
  }

  function invitationsAnswer(result) {
    var groups = groupsState();
    if (result.ok) {
      // Group's UUID, fee, inviter's UUID or `?`, inviter's name, group's name or `?`.
      groups.invites = listing(result.text).map(function (f) {
        var fee = parseInt(f[1], 10);
        return {
          id: (f[0] || '').toLowerCase(),
          fee: isNaN(fee) || fee < 0 ? 0 : fee,
          inviterId: unknown(f[2]) ? '' : f[2],
          inviter: f[3] || '',
          name: unknown(f[4]) ? '' : f[4]
        };
      });
      groups.invitesError = '';
    } else {
      groups.invites = null;
      groups.invitesError = result.text;
    }
  }

  // Asks for the groups and the invitations. While they are being asked
  // for, another ask is made once they are answered.
  function loadGroups(own) {
    var groups = groupsState();
    groups.busy = true;
    renderGroupList();
    return Promise.all([run(['group', 'list'], own), run(['group', 'invitations'], own)]).then(function (results) {
      if (state.groups !== groups) { return; }
      groups.busy = false;
      groupsListAnswer(results[0]);
      invitationsAnswer(results[1]);
      renderGroupList();
      renderGroupView();
      renderInvitations();
      if (groups.again) {
        groups.again = false;
        refreshGroups(true);
      }
    });
  }

  function refreshGroups(own) {
    if (!$('login').hidden) { return; }
    var groups = groupsState();
    if (groups.busy) { groups.again = true; return; }
    loadGroups(own);
  }

  // ---------------------------------------------------- the invitations

  function renderInvitations() {
    var groups = groupsState();
    var panel = $('groups-invitations');
    var list = $('groups-invitations-list');
    clear(list);
    var note = $('groups-invitations-note');
    // Not connected: nothing can be answered, and the list says so.
    var error = connected() ? groups.invitesError : '';
    note.textContent = error;
    show(note, error !== '');
    var invites = groups.invites || [];
    invites.forEach(function (invite) { list.appendChild(invitationNode(invite)); });
    show(panel, error !== '' || invites.length > 0);
    groupsMark();
  }

  function invitationNode(invite) {
    var groups = groupsState();
    var label = invite.name || invite.id;
    var row = el('li', 'invitation');
    var who = el('div', 'who');
    var title = el('div', 'title', label);
    if (invite.fee > 0) {
      title.appendChild(document.createTextNode(' '));
      title.appendChild(el('span', 'fee', 'L$' + invite.fee));
    }
    who.appendChild(title);
    who.appendChild(el('div', 'more', 'invited by ' + (invite.inviter || invite.inviterId || 'someone unknown') + (invite.name ? ' · ' + invite.id : '')));
    var payless = invite.fee > 0 && !canPay();
    if (payless) { who.appendChild(el('div', 'more error', 'Payments are not enabled for the web interface, so it cannot be accepted here.')); }
    row.appendChild(who);
    var acts = el('div', 'acts');
    var answer = groups.answering[invite.id];
    var busy = !!answer && answer.kind === 'wait';
    var buttons = [];
    if (!payless) {
      buttons.push(rowButton('Accept', function () { acceptInvitation(invite, label); }));
    }
    buttons.push(rowButton('Reject', function () { answerInvitation(invite, label, ['decline', 'group', invite.id], 'Rejecting…'); }));
    buttons.forEach(function (button) {
      button.disabled = busy;
      acts.appendChild(button);
    });
    row.appendChild(acts);
    if (answer) {
      var result = el('p', 'result' + (answer.kind ? ' ' + answer.kind : ''), answer.text);
      result.setAttribute('role', answer.kind === 'error' ? 'alert' : 'status');
      row.appendChild(result);
    }
    return row;
  }

  function acceptInvitation(invite, label) {
    if (invite.fee <= 0) {
      answerInvitation(invite, label, ['accept', 'group', invite.id], 'Accepting…');
      return;
    }
    if (!canPay()) { return; }
    var fee = String(invite.fee);
    var details = [
      'Group: ' + (invite.name || '(name not known)'),
      'Group UUID: ' + invite.id,
      'Invited by: ' + (invite.inviter || invite.inviterId || 'someone unknown')
    ];
    confirmPayment('Pay the fee of L$' + fee + ' to join ' + label + '?', details, fee).then(function (yes) {
      if (!yes) { return; }
      answerInvitation(invite, label, ['accept', 'group', invite.id, '--pay', fee], 'Paying L$' + fee + ' and joining…');
    });
  }

  // Accepts or rejects: the row says so, then shows the answer or the
  // error; the lists are asked for again.
  function answerInvitation(invite, label, args, doing) {
    var groups = groupsState();
    var now = groups.answering[invite.id];
    if (now && now.kind === 'wait') { return; }
    groups.answering[invite.id] = { text: doing, kind: 'wait' };
    renderInvitations();
    run(args, false).then(function (result) {
      if (state.groups !== groups) { return; }
      if (result.ok) {
        delete groups.answering[invite.id];
        // The row goes once the invitation no longer waits: the answer
        // is shown where answers are.
        notify('ginv:' + invite.id, (args[0] === 'accept' ? 'Accept ' : 'Reject ') + label, result.text || 'done', false);
      } else {
        groups.answering[invite.id] = { text: result.text, kind: 'error' };
      }
      renderInvitations();
      refreshGroups(true);
    });
  }

  // --------------------------------------------------------------- Leave

  // Leaves the chosen group once the operator has confirmed it; the list
  // shows the change when the bot says the groups changed.
  function leaveGroup() {
    var id = groupsState().chosen;
    if (!id || !listedGroup(id)) { return; }
    var button = $('group-leave');
    if (button.disabled) { return; }
    var label = groupName(id);
    var details = [
      'Group: ' + label,
      'Group UUID: ' + id,
      'Joining it again may take an invitation or a fee.'
    ];
    confirmAction('Leave the group', 'Leave ' + label + '?', details, 'Leave').then(function (yes) {
      if (!yes) { return; }
      act(button, 'gleave:' + id, 'Leave ' + label, ['group', 'leave', id], false).then(function (result) {
        if (result && result.ok) { refreshGroups(true); }
      });
    });
  }

  // --------------------------------------------------- Info and Invite

  var INFO_NAMES = {
    name: 'Name', group: 'UUID', founder: 'Founder', fee: 'Fee', enrollment: 'Enrollment',
    members: 'Members', roles: 'Roles', listed: 'Listed', mature: 'Mature', member: 'Member',
    title: 'Title', charter: 'Charter'
  };

  // `name: value` lines, each value escaped.
  function infoLines(text) {
    return text.split('\n').filter(function (line) { return line !== ''; }).map(function (line) {
      var colon = line.indexOf(': ');
      if (colon < 0) { return [line.replace(/:$/, ''), '']; }
      return [line.slice(0, colon), unescapeField(line.slice(colon + 2))];
    });
  }

  function renderGroupInfo() {
    var groups = groupsState();
    var group = groups.chosen ? groups.known[groups.chosen] : null;
    var box = $('group-info-box');
    clear(box);
    var info = group ? group.info : null;
    show(box, !!info);
    if (!info) { return; }
    var head = el('div', 'head');
    head.appendChild(el('strong', null, 'Group info'));
    var close = rowButton('Close', function () {
      group.info = null;
      renderGroupInfo();
    });
    close.classList.add('small');
    head.appendChild(close);
    box.appendChild(head);
    if (info.busy) { box.appendChild(el('p', 'made', 'Asking the grid…')); return; }
    if (info.error) {
      var error = el('p', 'result error', info.error);
      error.setAttribute('role', 'alert');
      box.appendChild(error);
      return;
    }
    var list = el('dl');
    info.lines.forEach(function (pair) {
      list.appendChild(el('dt', null, INFO_NAMES[pair[0]] || pair[0]));
      list.appendChild(el('dd', pair[0] === 'charter' ? 'charter' : null, pair[1]));
    });
    box.appendChild(list);
  }

  function askGroupInfo() {
    var groups = groupsState();
    var id = groups.chosen;
    var button = $('group-info');
    if (!id || button.disabled) { return; }
    var group = ensureGroup(id);
    button.disabled = true;
    button.classList.add('busy');
    group.info = { busy: true, error: '', lines: [] };
    renderGroupInfo();
    run(['group', 'info', id], false).then(function (result) {
      button.disabled = false;
      button.classList.remove('busy');
      if (state.groups !== groups) { return; }
      group.info = result.ok ? { busy: false, error: '', lines: infoLines(result.text) } : { busy: false, error: result.text, lines: [] };
      if (groups.chosen === id) { renderGroupInfo(); }
    });
  }

  function renderGroupInvite() {
    var groups = groupsState();
    var group = groups.chosen ? groups.known[groups.chosen] : null;
    var listed = group ? listedGroup(group.id) : null;
    var invite = group && listed && listed.flags.indexOf('invite') >= 0 ? group.invite : null;
    var form = $('group-invite-form');
    show(form, !!invite);
    if (!invite) { return; }
    var box = $('group-invite-who');
    box.value = invite.text;
    box.classList.remove('bad');
    var chosen = $('group-invite-chosen');
    var isChosen = invite.chosen && invite.chosen.shown === invite.text;
    chosen.textContent = isChosen ? 'Invites ' + invite.chosen.shown + ' by the UUID ' + invite.chosen.id + '.' : '';
    show(chosen, !!isChosen);
    var result = $('group-invite-result');
    setResult(result, invite.result ? invite.result.text : '', invite.result ? invite.result.kind : '');
    renderInviteFriends();
  }

  function renderInviteFriends() {
    var list = $('group-invite-friends');
    clear(list);
    var groups = groupsState();
    var group = groups.chosen ? groups.known[groups.chosen] : null;
    (state.friends || []).forEach(function (friend) {
      list.appendChild(friendNode(friend, function () {
        if (!group || !group.invite) { return; }
        group.invite.text = friend.label;
        group.invite.chosen = { shown: friend.label, id: friend.id };
        renderGroupInvite();
        $('group-invite-send').focus();
      }));
    });
    var note = $('group-invite-friends-note');
    var text = state.friends && state.friends.length === 0 ? 'No friends.' : !state.friends ? 'Reading the friend list…' : '';
    note.textContent = text;
    show(note, text !== '');
  }

  function sendGroupInvite() {
    var groups = groupsState();
    var id = groups.chosen;
    var group = id ? groups.known[id] : null;
    if (!group || !group.invite) { return; }
    var box = $('group-invite-who');
    var typed = box.value.trim();
    box.classList.toggle('bad', typed === '');
    if (typed === '') { box.focus(); return; }
    group.invite.text = box.value;
    var chosen = group.invite.chosen && group.invite.chosen.shown === box.value ? group.invite.chosen : null;
    var who = chosen ? chosen.id : typed;
    var label = chosen ? chosen.shown : typed;
    var button = $('group-invite-send');
    if (button.disabled) { return; }
    button.disabled = true;
    button.classList.add('busy');
    group.invite.result = { text: 'Inviting ' + label + '…', kind: 'wait' };
    renderGroupInvite();
    run(['group', 'invite', id, who], false).then(function (result) {
      button.disabled = false;
      button.classList.remove('busy');
      if (state.groups !== groups || !group.invite) { return; }
      group.invite.result = { text: result.ok ? (result.text || 'Invited.') : result.text, kind: result.ok ? 'ok' : 'error' };
      if (groups.chosen === id) {
        group.invite.text = $('group-invite-who').value;
        renderGroupInvite();
      }
    });
  }

  function sendGroupChat() {
    var groups = groupsState();
    var id = groups.chosen;
    var input = $('group-text');
    var text = input.value;
    if (!id || !listedGroup(id) || text.trim() === '') { return; }
    act($('group-button'), 'gchat:' + id, 'Message to ' + groupName(id), ['group', 'chat', id, text], true).then(function (result) {
      if (!result || !result.ok) { input.focus(); return; }
      // The line shows when its record arrives.
      var group = groups.known[id];
      if (groups.chosen === id && input.value === text) { input.value = ''; }
      if (group && group.draft === text) { group.draft = ''; }
      if (groups.chosen === id) { input.focus(); }
    });
  }

  function wireGroups() {
    $('groups-refresh').addEventListener('click', function () {
      var button = $('groups-refresh');
      var groups = groupsState();
      if (button.disabled || groups.busy) { return; }
      button.disabled = true;
      button.classList.add('busy');
      // The operator asked: ordinary commands.
      loadGroups(false).then(function () {
        button.disabled = false;
        button.classList.remove('busy');
      });
    });
    $('group-form').addEventListener('submit', function (event) {
      event.preventDefault();
      sendGroupChat();
    });
    $('group-activate').addEventListener('click', function () {
      var id = groupsState().chosen;
      if (!id) { return; }
      act($('group-activate'), 'gact:' + id, 'Activate ' + groupName(id), ['group', 'activate', id], false);
    });
    $('group-info').addEventListener('click', askGroupInfo);
    $('group-leave').addEventListener('click', leaveGroup);
    $('group-invite').addEventListener('click', function () {
      var groups = groupsState();
      var group = groups.chosen ? groups.known[groups.chosen] : null;
      if (!group) { return; }
      if (group.invite) { group.invite.text = $('group-invite-who').value; }
      else { group.invite = { text: '', chosen: null, result: null }; }
      renderGroupInvite();
      $('group-invite-who').focus();
      refreshFriends(true);
    });
    $('group-invite-close').addEventListener('click', function () {
      var groups = groupsState();
      var group = groups.chosen ? groups.known[groups.chosen] : null;
      if (group) { group.invite = null; }
      renderGroupInvite();
    });
    $('group-invite-form').addEventListener('submit', function (event) {
      event.preventDefault();
      sendGroupInvite();
    });
    $('group-invite-who').addEventListener('input', function () {
      var groups = groupsState();
      var group = groups.chosen ? groups.known[groups.chosen] : null;
      var box = $('group-invite-who');
      box.classList.remove('bad');
      if (group && group.invite) { group.invite.text = box.value; }
      show($('group-invite-chosen'), !!(group && group.invite && group.invite.chosen && group.invite.chosen.shown === box.value));
    });
    var log = $('group-log');
    log.addEventListener('scroll', function () {
      if (state.tab !== 'groups') { return; }
      groupsState().follow = atEnd(log);
      if (groupsState().follow) { show($('group-new'), false); }
    });
    $('group-new').addEventListener('click', function () {
      groupsState().follow = true;
      scrollToEnd(log);
      show($('group-new'), false);
    });
    document.addEventListener('visibilitychange', function () {
      if (document.visibilityState === 'visible' && state.tab === 'groups') {
        groupSeen();
        renderGroupList();
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

  // ----------------------------------------------------------------- money

  // The web interface may pay while the status has a `limit` line: then
  // there is a Money tab and there are Pay buttons, and otherwise none.
  function canPay() {
    return !!state.status && state.status.limit !== undefined;
  }

  // While RLV holds back the descriptions of payments the status says so,
  // and the form says so in place of the description box.
  function descriptionBlocked() {
    return !!state.status && state.status['pay description'] !== undefined;
  }

  function renderDescription() {
    var blocked = descriptionBlocked();
    show($('pay-description-label'), !blocked);
    show($('pay-description-blocked'), blocked);
  }

  // The bytes `text` takes in UTF-8.
  function utf8Length(text) {
    return new TextEncoder().encode(text).length;
  }

  function renderImPay() {
    var conv = state.conv ? state.convs[state.conv] : null;
    show($('im-pay'), canPay() && !!conv && conv.kind === 'avatar');
  }

  function renderPayControls() {
    var can = canPay();
    show($('tab-link-money'), can);
    var controls = document.querySelectorAll('.pay-control');
    for (var i = 0; i < controls.length; i++) { show(controls[i], can); }
    renderImPay();
    renderInvitations();
    if (!can) { closeDialog(); }
    // The Money tab's address shows the World tab without it, and the
    // Money tab again once it is there.
    if ((state.tab === 'money') !== (tabOfAddress() === 'money')) { route(); }
  }

  // `L$750 (renews to L$1000 on 2026-12-08T12:00:00Z)`, `L$1000`, `unlimited`.
  function parseLimit(text) {
    var found = /^(.*?) \(renews to (\S+) on (\S+)\)$/.exec(text);
    return found ? { left: found[1], renewsTo: found[2], on: found[3] } : { left: text, renewsTo: '', on: '' };
  }

  // A time of the bot's, in UTC, as the browser's time zone has it.
  function localTime(iso) {
    var date = new Date(iso);
    if (isNaN(date.getTime())) { return iso; }
    try {
      return date.toLocaleString(undefined, { dateStyle: 'long', timeStyle: 'short' });
    } catch (e) {
      return date.toLocaleString();
    }
  }

  function renderMoneyHeader() {
    var status = state.status || {};
    $('money-avatar').textContent = status.avatar || 'not known yet';
    var balance = status.balance;
    $('money-balance').textContent = !balance || balance === 'unknown' ? 'unknown' : balance;
    var limit = parseLimit(status.limit || '');
    $('money-left').textContent = limit.left === 'unlimited' ? 'no limit' : (limit.left || '?');
    var renews = $('money-renews');
    renews.textContent = limit.renewsTo ? '(renews to ' + limit.renewsTo + ' on ' + localTime(limit.on) + ')' : '';
    show(renews, limit.renewsTo !== '');
  }

  function setResult(node, text, kind) {
    node.textContent = text;
    node.className = 'result' + (kind ? ' ' + kind : '');
    show(node, text !== '');
  }

  // An amount box: its amount, or null when it holds no whole number
  // greater than zero written in digits, in which case the box is marked.
  function amountOf(input) {
    var text = input.value.trim();
    var good = /^\d+$/.test(text) && /[1-9]/.test(text);
    input.classList.toggle('bad', !good);
    return good ? text.replace(/^0+/, '') : null;
  }

  // ------------------------------------------------- the confirmation

  var confirming = null;  // resolves the confirmation that is open

  // Asks the operator before an action: true only when the button that
  // does it is pressed; cancelling, Escape and a click outside do nothing.
  // The cancelling button has the focus.
  function confirmAction(title, question, details, yes) {
    var dialog = $('confirm-dialog');
    if (typeof dialog.showModal !== 'function') {
      return Promise.resolve(window.confirm(question + '\n' + details.join('\n')));
    }
    closeDialog();
    $('confirm-dialog-title').textContent = title;
    $('confirm-dialog-text').textContent = question;
    var more = $('confirm-dialog-more');
    clear(more);
    details.forEach(function (line) { more.appendChild(el('div', null, line)); });
    $('confirm-dialog-yes').textContent = yes;
    return new Promise(function (resolve) {
      confirming = resolve;
      dialog.returnValue = '';
      dialog.showModal();
      $('confirm-dialog-cancel').focus();
    });
  }

  // Asks the operator before a payment. Nothing is paid unless the paying
  // button is pressed.
  function confirmPayment(question, details, amount) {
    return confirmAction('Confirm the payment', question, details, 'Pay L$' + amount);
  }

  function closeDialog() {
    var dialog = $('confirm-dialog');
    if (dialog.open) { dialog.close('cancel'); }
  }

  function wireDialog() {
    var dialog = $('confirm-dialog');
    dialog.addEventListener('close', function () {
      var resolve = confirming;
      confirming = null;
      if (resolve) { resolve(dialog.returnValue === 'yes'); }
    });
    $('confirm-dialog-cancel').addEventListener('click', function () { dialog.close('cancel'); });
    $('confirm-dialog-yes').addEventListener('click', function () { dialog.close('yes'); });
    // A click on the backdrop is a click on the dialog itself, outside its box.
    dialog.addEventListener('click', function (event) {
      if (event.target !== dialog) { return; }
      var box = dialog.getBoundingClientRect();
      var inside = event.clientX >= box.left && event.clientX <= box.right && event.clientY >= box.top && event.clientY <= box.bottom;
      if (!inside) { dialog.close('cancel'); }
    });
  }

  // ------------------------------------------------ the Send money form

  // A payee chosen from a list: the box shows its name, and it is paid by
  // `pay` (its UUID, or its name for a list entry without one).
  function choosePayee(shown, pay, id) {
    state.payee = { shown: shown, pay: pay, id: id || '' };
    var box = $('pay-payee');
    box.value = shown;
    box.classList.remove('bad');
    var note = $('pay-chosen');
    note.textContent = id ? 'Pays ' + shown + ' by the UUID ' + id + '.' : 'Pays ' + shown + ' by name.';
    show(note, true);
    if (state.tab === 'money') { $('pay-amount').focus(); }
  }

  function sendFromForm() {
    if (state.paying) { return; }
    var payeeBox = $('pay-payee');
    var amountBox = $('pay-amount');
    var descriptionBox = $('pay-description');
    var typed = payeeBox.value.trim();
    payeeBox.classList.toggle('bad', typed === '');
    var amount = amountOf(amountBox);
    var description = descriptionBlocked() ? '' : descriptionBox.value.trim();
    var tooLong = utf8Length(description) > 127;
    descriptionBox.classList.toggle('bad', tooLong);
    if (typed === '' || amount === null || tooLong) { return; }
    var chosen = state.payee && state.payee.shown === payeeBox.value ? state.payee : null;
    var to = chosen ? chosen.pay : typed;
    var label = chosen ? chosen.shown : typed;
    var details = chosen ? (chosen.id ? ['UUID: ' + chosen.id] : ['By the name the list gives.']) : ['As typed.'];
    if (description !== '') { details.push('Description: ' + description); }
    confirmPayment('Pay L$' + amount + ' to ' + label + '?', details, amount).then(function (yes) {
      if (!yes || state.paying) { return; }
      var button = $('pay-button');
      state.paying = true;
      button.disabled = true;
      button.classList.add('busy');
      setResult($('pay-result'), 'Paying L$' + amount + ' to ' + label + '; the grid confirms it within 15 seconds…', 'wait');
      var args = ['pay', amount, 'avatar', to];
      if (description !== '') { args.push('-d', description); }
      run(args, false).then(function (result) {
        state.paying = false;
        button.disabled = false;
        button.classList.remove('busy');
        if (result.ok) {
          amountBox.value = '';
          descriptionBox.value = '';
          setResult($('pay-result'), result.text || 'Paid.', 'ok');
        } else {
          setResult($('pay-result'), result.text, 'error');
        }
      });
    });
  }

  // ------------------------------------------------- the lists of payees

  // UUID or `?`, login name, `whitelist`, `unlimited` or both.
  function payeesAnswer(result) {
    if (result.ok) {
      state.payees = listing(result.text).map(function (f) {
        var id = UUID.test(f[0] || '') ? f[0].toLowerCase() : '';
        return { id: id, name: f[1] || id, lists: (f[2] || '').split(',') };
      }).filter(function (payee) { return payee.name !== ''; });
      state.payeesError = '';
    } else {
      state.payees = null;
      state.payeesError = result.text;
    }
    renderPayees();
  }

  function loadPayees(own) {
    state.payeesBusy = true;
    return run(['payees'], own).then(function (result) {
      state.payeesBusy = false;
      payeesAnswer(result);
    });
  }

  function refreshPayees(own) {
    if (state.payeesBusy || !$('login').hidden) { return; }
    loadPayees(own);
  }

  function renderPayees() {
    ['whitelist', 'unlimited'].forEach(function (name) {
      var list = $('money-' + name);
      clear(list);
      var entries = (state.payees || []).filter(function (payee) { return payee.lists.indexOf(name) >= 0; });
      entries.forEach(function (payee) {
        var item = el('li');
        item.appendChild(el('span', 'label', payee.name));
        if (!payee.id) { item.appendChild(el('span', 'more', 'by name')); }
        item.addEventListener('click', function () { choosePayee(payee.name, payee.id || payee.name, payee.id); });
        list.appendChild(item);
      });
      // No heading for a list without entries.
      show($('money-' + name + '-heading'), entries.length > 0);
    });
    var note = $('money-payees-note');
    note.textContent = state.payeesError ? 'The payment lists could not be read: ' + state.payeesError : '';
    note.classList.toggle('error', !!state.payeesError);
    show(note, !!state.payeesError);
  }

  // ------------------------------------------------- the transactions

  var KINDS = {
    paid: 'Paid',
    unconfirmed: 'Sent, not confirmed by the grid',
    failed: 'The payment failed',
    received: 'Received',
    spent: 'Spent'
  };

  // A row of the transactions file, as the event stream gives it.
  function transaction(f) {
    return {
      time: f[0] || '',
      kind: f[1] || '',
      amount: f[2] || '',
      balance: f[3] || '',
      counted: f[4] || '',
      type: f[5] || '',
      partyId: f[6] || '',
      avatarName: f[7] || '',
      displayName: f[8] || '',
      objectName: f[9] || '',
      objectId: f[10] || '',
      text: f[11] || ''
    };
  }

  function transactionNode(tx) {
    var odd = tx.kind === 'failed' || tx.kind === 'unconfirmed';
    var item = el('li', 'tx' + (odd ? ' ' + tx.kind : ''));
    var who = el('div', 'who');
    var head = el('div', 'title');
    head.appendChild(el('span', 'kind', KINDS[tx.kind] || tx.kind));
    head.appendChild(document.createTextNode(odd ? ': ' : ' '));
    head.appendChild(el('span', 'amount', 'L$' + tx.amount));
    who.appendChild(head);
    var parts = [];
    var party = avatarLabel(tx.avatarName, tx.displayName) || tx.partyId;
    if (party) { parts.push((tx.kind === 'received' ? 'from ' : tx.kind === 'failed' ? 'with ' : 'to ') + party); }
    if (tx.objectName || tx.objectId) { parts.push('object ' + (tx.objectName || tx.objectId)); }
    if (parts.length) { who.appendChild(el('div', null, parts.join(' · '))); }
    if (tx.text) { who.appendChild(el('div', 'more', tx.text)); }
    if (tx.balance !== '') { who.appendChild(el('div', 'more', 'Balance after: L$' + tx.balance)); }
    item.appendChild(who);
    var epoch = Date.parse(tx.time);
    item.appendChild(isNaN(epoch) ? el('span', 'time', tx.time) : timeNode(epoch / 1000));
    return item;
  }

  // Newest first.
  function addTransaction(tx) {
    var list = $('money-list');
    state.money.push(tx);
    list.insertBefore(transactionNode(tx), list.firstChild);
    while (state.money.length > MAX_TRANSACTIONS) {
      state.money.shift();
      if (list.lastChild) { list.removeChild(list.lastChild); }
    }
    show($('money-none'), false);
  }

  function renderTransactions() {
    var list = $('money-list');
    clear(list);
    for (var i = state.money.length - 1; i >= 0; i--) { list.appendChild(transactionNode(state.money[i])); }
    show($('money-none'), state.money.length === 0);
  }

  // ------------------------------------------- paying from the object scan

  // The answer of `prices`: `object`, `owner`, `prices` and `other amounts`.
  function pricesOf(text) {
    var info = { object: '', owner: '', ownerId: '', group: false, prices: [], other: false, suggested: '' };
    text.split('\n').forEach(function (line) {
      var colon = line.indexOf(': ');
      if (colon < 0) { return; }
      var name = line.slice(0, colon), value = line.slice(colon + 2).trim();
      if (name === 'object') {
        info.object = unescapeField(value);
      } else if (name === 'owner') {
        if (/^group /.test(value)) { info.group = true; info.ownerId = value.slice(6).trim(); }
        else if (UUID.test(value)) { info.ownerId = value; }
        else { info.owner = value; }
      } else if (name === 'prices') {
        info.prices = value === 'none' ? [] : value.split(/,\s*/).filter(function (amount) { return /^\d+$/.test(amount); });
      } else if (name === 'other amounts') {
        var other = /^yes(?:, suggested (\d+))?$/.exec(value);
        info.other = !!other;
        info.suggested = other && other[1] ? other[1] : '';
      }
    });
    return info;
  }

  function ownerText(info) {
    if (info.group) { return 'A group owns it: ' + info.ownerId; }
    return 'Owner: ' + (info.owner || info.ownerId || 'unknown');
  }

  function askPrices(button, area, id, rowName) {
    if (button.disabled) { return; }
    button.disabled = true;
    button.classList.add('busy');
    clear(area);
    area.appendChild(el('p', 'made', 'Asking the object what it takes…'));
    run(['prices', id], false).then(function (result) {
      button.disabled = false;
      button.classList.remove('busy');
      clear(area);
      if (!result.ok) {
        var error = el('p', 'result error', result.text);
        error.setAttribute('role', 'alert');
        area.appendChild(error);
        return;
      }
      renderPrices(area, id, pricesOf(result.text), rowName);
    });
  }

  function renderPrices(area, id, info, rowName) {
    var objectName = info.object || rowName;
    area.appendChild(el('div', 'more', ownerText(info)));
    if (info.prices.length === 0 && !info.other) {
      area.appendChild(el('p', 'made', 'The object offers no amount.'));
      return;
    }
    var form = el('form', 'fields');
    form.noValidate = true;
    form.setAttribute('autocomplete', 'off');
    var amounts = el('select', 'pay-amounts');
    amounts.setAttribute('aria-label', 'Amount');
    info.prices.forEach(function (amount) {
      var option = el('option', null, 'L$' + amount);
      option.value = amount;
      amounts.appendChild(option);
    });
    if (info.other) {
      var another = el('option', null, 'Another amount');
      another.value = OTHER_AMOUNT;
      amounts.appendChild(another);
    }
    form.appendChild(amounts);
    // A box for another amount only for an object that takes one, shown
    // while that entry is chosen.
    var other = null;
    if (info.other) {
      var otherLabel = el('label', null, 'L$ ');
      other = el('input', 'pay-other');
      other.setAttribute('inputmode', 'numeric');
      other.setAttribute('autocomplete', 'off');
      other.setAttribute('aria-label', 'Another amount');
      other.size = 7;
      other.value = info.suggested;
      otherLabel.appendChild(other);
      form.appendChild(otherLabel);
      var sync = function () { show(otherLabel, amounts.value === OTHER_AMOUNT); };
      amounts.addEventListener('change', function () {
        sync();
        if (amounts.value === OTHER_AMOUNT) { other.focus(); }
      });
      other.addEventListener('input', function () { other.classList.remove('bad'); });
      sync();
    }
    var send = el('button', 'pay-send', 'Pay the object');
    send.type = 'submit';
    form.appendChild(send);
    area.appendChild(form);
    var result = el('p', 'result');
    result.setAttribute('role', 'status');
    show(result, false);
    area.appendChild(result);
    form.addEventListener('submit', function (event) {
      event.preventDefault();
      if (send.disabled) { return; }
      var amount = other && amounts.value === OTHER_AMOUNT ? amountOf(other) : amounts.value;
      if (!amount) { return; }
      var details = ['Object UUID: ' + id, ownerText(info)];
      confirmPayment('Pay L$' + amount + ' to the object ' + objectName + '?', details, amount).then(function (yes) {
        if (!yes || send.disabled) { return; }
        send.disabled = true;
        send.classList.add('busy');
        setResult(result, 'Paying L$' + amount + '; the grid confirms it within 15 seconds…', 'wait');
        run(['pay', amount, 'object', id], false).then(function (answer) {
          send.disabled = false;
          send.classList.remove('busy');
          setResult(result, answer.ok ? (answer.text || 'Paid.') : answer.text, answer.ok ? 'ok' : 'error');
        });
      });
    });
  }

  function wireMoney() {
    wireDialog();
    $('pay-form').addEventListener('submit', function (event) {
      event.preventDefault();
      sendFromForm();
    });
    // Typing makes what is typed the payee again.
    $('pay-payee').addEventListener('input', function () {
      state.payee = null;
      show($('pay-chosen'), false);
      $('pay-payee').classList.remove('bad');
    });
    $('pay-amount').addEventListener('input', function () { $('pay-amount').classList.remove('bad'); });
    $('pay-description').addEventListener('input', function () { $('pay-description').classList.remove('bad'); });
    $('money-lists-refresh').addEventListener('click', function () {
      var button = $('money-lists-refresh');
      if (button.disabled) { return; }
      button.disabled = true;
      button.classList.add('busy');
      // The operator asked: ordinary commands.
      var jobs = [];
      if (!state.friendsBusy) {
        state.friendsBusy = true;
        jobs.push(run(['friends'], false).then(function (result) {
          state.friendsBusy = false;
          friendsAnswer(result);
        }));
      }
      if (!state.payeesBusy) { jobs.push(loadPayees(false)); }
      Promise.all(jobs).then(function () {
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
    renderGroupList();
    renderGroupView();
    renderInvitations();
    renderInventory();
    renderTransactions();
    renderPayees();
    renderDescription();
    clear($('avatars-list'));
    clear($('objects-list'));
    show($('avatars-made'), false);
    show($('objects-made'), false);
    clear($('console-out'));
    show($('mark-chat'), false);
    var inputs = document.querySelectorAll('#app input:not([type="radio"]):not([type="checkbox"])');
    for (var i = 0; i < inputs.length; i++) { inputs[i].value = ''; inputs[i].classList.remove('bad'); }
    show($('pay-chosen'), false);
    setResult($('pay-result'), '', '');
    renderPayControls();
  }

  function start() {
    state = freshState();
    wireLogin();
    wireWorld();
    wireChat();
    wireIm();
    wireGroups();
    wireInventory();
    wireMoney();
    wireConsole();
    wireSettings();
    renderEverything();
    window.addEventListener('hashchange', route);
    // The lists the page keeps up to date, while their tab is looked at.
    window.setInterval(function () {
      if (state.tab === 'chat' && document.visibilityState === 'visible' && !$('app').hidden && !state.lost) { refreshNearby(true); }
    }, NEARBY_EVERY);
    window.setInterval(function () {
      if ((state.tab === 'im' || state.tab === 'money') && document.visibilityState === 'visible' && !$('app').hidden && !state.lost) { refreshFriends(true); }
    }, FRIENDS_EVERY);
    begin();
  }

  if (document.readyState === 'loading') { document.addEventListener('DOMContentLoaded', start); }
  else { start(); }
})();
