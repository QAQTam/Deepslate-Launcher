/**
 * Brainstorm Companion — Client helper
 * Injected into every page by the server.
 */
(function() {
  'use strict';

  // ========== WebSocket ==========
  const wsProto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  let ws;
  let reconnectTimer;

  function connect() {
    if (ws && ws.readyState === WebSocket.OPEN) return;
    ws = new WebSocket(wsProto + '//' + location.host);
    ws.onopen = function() {
      if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
      updateStatus(true);
    };
    ws.onclose = function() {
      updateStatus(false);
      if (!reconnectTimer) reconnectTimer = setTimeout(connect, 2000);
    };
    ws.onerror = function() { /* onclose will fire */ };
    ws.onmessage = function(e) {
      try {
        var msg = JSON.parse(e.data);
        if (msg.type === 'reload') location.reload();
      } catch (err) {}
    };
  }

  function send(msg) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    }
  }

  function updateStatus(connected) {
    var el = document.querySelector('.status');
    if (!el) {
      el = document.createElement('div');
      el.className = 'status';
      var header = document.querySelector('.header');
      if (header) header.appendChild(el);
      else return;
    }
    el.textContent = connected ? 'Connected' : 'Reconnecting…';
    el.style.setProperty('--status-color', connected ? 'var(--success)' : 'var(--warning)');
  }

  // ========== Option Selection ==========
  window.toggleSelect = function(el) {
    var container = el.closest('.options, .cards');
    var multi = container && container.hasAttribute('data-multiselect');

    if (!multi) {
      var siblings = (container || el.parentElement).querySelectorAll('.option.selected, .card.selected');
      siblings.forEach(function(s) { if (s !== el) s.classList.remove('selected'); });
    }

    el.classList.toggle('selected');

    var choice = el.getAttribute('data-choice');
    var text = (el.querySelector('.content h3, .card-body h3') || {}).textContent || choice;
    send({ type: 'click', choice: choice, text: text, timestamp: Math.floor(Date.now() / 1000) });

    updateSelectedText();
  };

  function updateSelectedText() {
    var header = document.querySelector('.header span.selected-text');
    if (!header) {
      header = document.createElement('span');
      header.className = 'selected-text';
      var hdr = document.querySelector('.header');
      if (hdr) hdr.appendChild(header);
    }
    if (!header) return;
    var selected = document.querySelectorAll('.option.selected, .card.selected');
    if (selected.length > 0) {
      var labels = [];
      selected.forEach(function(s) {
        var choice = s.getAttribute('data-choice');
        if (choice) labels.push(choice.toUpperCase());
      });
      header.textContent = labels.length > 0 ? 'Selected: ' + labels.join(', ') : '';
    } else {
      header.textContent = '';
    }
  }

  // ========== Init ==========
  connect();
})();
