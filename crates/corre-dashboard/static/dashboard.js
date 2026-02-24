/* Corre Dashboard — XP-themed SSE client */
(function () {
    "use strict";

    var TOKEN = window.__DASHBOARD_TOKEN || "";
    var CONFIG = window.__CONFIG || {};
    var MAX_LOG_ROWS = 500;
    var capabilities = {};       // name -> state object
    var knownCapNames = [];
    var levelFilters = { DEBUG: true, INFO: true, WARN: true, ERROR: true };
    var capFilter = "";
    var searchText = "";
    var autoScroll = true;
    var historicalMode = false;
    var logContainer = document.getElementById("log-container");
    var logTbody = document.getElementById("log-tbody");
    var capTbody = document.getElementById("capabilities-tbody");
    var jumpBtn = document.getElementById("log-jump");

    // --- SSE Connection ---
    function connect() {
        var url = "/api/dashboard/events?token=" + encodeURIComponent(TOKEN);
        var es = new EventSource(url);

        es.onmessage = function (e) {
            try {
                var event = JSON.parse(e.data);
                handleEvent(event);
            } catch (err) {
                console.error("Failed to parse SSE event:", err);
            }
        };

        es.onerror = function () {
            es.close();
            setTimeout(connect, 3000);
        };
    }

    function handleEvent(event) {
        switch (event.type) {
            case "CapabilityUpdate":
                updateCapability(event);
                break;
            case "LogLine":
                if (!historicalMode) {
                    appendLog(event.capability, event.entry);
                }
                break;
            case "SystemMetrics":
                updateMetrics(event);
                break;
        }
    }

    // --- Capabilities Table ---
    function updateCapability(state) {
        capabilities[state.name] = state;
        if (knownCapNames.indexOf(state.name) === -1) {
            knownCapNames.push(state.name);
            updateCapFilter();
        }
        renderCapabilities();
    }

    function renderCapabilities() {
        var html = "";
        var names = Object.keys(capabilities).sort();
        for (var i = 0; i < names.length; i++) {
            var s = capabilities[names[i]];
            html += renderCapRow(s);
        }
        if (names.length === 0) {
            html = '<tr><td colspan="7" class="loading-cell">No capabilities configured</td></tr>';
        }
        capTbody.innerHTML = html;

        // Bind run-now buttons
        var btns = capTbody.querySelectorAll(".run-now-btn");
        for (var j = 0; j < btns.length; j++) {
            btns[j].addEventListener("click", onRunNow);
        }
    }

    function renderCapRow(s) {
        var statusClass = "status-" + s.status;
        var lastRun = s.last_completed ? formatTime(s.last_completed) : "-";
        var duration = s.last_duration_secs != null ? s.last_duration_secs.toFixed(1) + "s" : "-";
        var articles = s.articles_produced != null ? s.articles_produced : "-";
        var disabled = s.status === "running" ? "disabled" : "";

        var progressHtml = "";
        if (s.status === "running" && s.progress_pct != null) {
            progressHtml = '<div class="cap-progress-track"><div class="cap-progress-fill" style="width:' +
                s.progress_pct + '%"></div></div>' +
                '<span class="cap-phase">' + escapeHtml(s.phase) + " " + s.progress_pct + "%</span>";
        }

        var errorHtml = "";
        if (s.last_error) {
            errorHtml = '<div class="cap-error" title="' + escapeAttr(s.last_error) + '">' +
                escapeHtml(s.last_error.substring(0, 100)) + '</div>';
        }

        return '<tr>' +
            '<td><span class="status-icon ' + statusClass + '"></span></td>' +
            '<td><span class="cap-name">' + escapeHtml(s.name) + '</span>' + progressHtml + errorHtml + '</td>' +
            '<td>' + escapeHtml(s.schedule) + '</td>' +
            '<td>' + lastRun + '</td>' +
            '<td>' + duration + '</td>' +
            '<td>' + articles + '</td>' +
            '<td><button class="xp-button run-btn run-now-btn" data-name="' + escapeAttr(s.name) + '" ' + disabled + '>Run Now</button></td>' +
            '</tr>';
    }

    function onRunNow(e) {
        var name = e.target.getAttribute("data-name");
        e.target.disabled = true;
        fetch("/api/dashboard/run/" + encodeURIComponent(name) + "?token=" + encodeURIComponent(TOKEN), {
            method: "POST"
        }).then(function (resp) {
            if (!resp.ok) {
                return resp.text().then(function (t) { alert("Run failed: " + t); });
            }
        }).catch(function (err) {
            alert("Network error: " + err);
        });
    }

    // --- System Metrics ---
    function updateMetrics(m) {
        var cpuPct = m.cpu_usage_percent.toFixed(1);
        document.getElementById("cpu-bar").style.width = cpuPct + "%";
        document.getElementById("cpu-value").textContent = cpuPct + "%";

        var procCpuPct = m.process_cpu_percent.toFixed(1);
        document.getElementById("proc-cpu-bar").style.width = Math.min(procCpuPct, 100) + "%";
        document.getElementById("proc-cpu-value").textContent = procCpuPct + "%";

        var memPct = m.memory_total_mb > 0 ? (m.memory_used_mb / m.memory_total_mb * 100).toFixed(1) : "0";
        document.getElementById("memory-bar").style.width = memPct + "%";
        document.getElementById("memory-value").textContent = m.memory_used_mb + " / " + m.memory_total_mb + " MB";

        var procMemPct = m.memory_total_mb > 0 ? (m.process_memory_mb / m.memory_total_mb * 100).toFixed(1) : "0";
        document.getElementById("proc-memory-bar").style.width = procMemPct + "%";
        document.getElementById("proc-memory-value").textContent = m.process_memory_mb + " MB";

        document.getElementById("uptime-value").textContent = formatDuration(m.uptime_secs);
    }

    // --- Log Viewer ---
    function appendLog(capability, entry) {
        var row = document.createElement("tr");
        row.className = "log-row log-row-" + entry.level;
        row.setAttribute("data-level", entry.level);
        row.setAttribute("data-cap", capability);

        var ts = formatTimestamp(entry.timestamp);
        var fullTs = entry.timestamp;

        row.innerHTML =
            '<td class="col-ts" title="' + escapeAttr(fullTs) + '">' + ts + '</td>' +
            '<td><span class="level-badge level-' + entry.level + '">' + entry.level + '</span></td>' +
            '<td class="col-cap">' + escapeHtml(capability) + '</td>' +
            '<td class="col-msg">' + escapeHtml(entry.message) + '</td>';

        logTbody.appendChild(row);

        // Prune old rows
        while (logTbody.children.length > MAX_LOG_ROWS) {
            logTbody.removeChild(logTbody.firstChild);
        }

        applyRowFilter(row);

        if (autoScroll) {
            logContainer.scrollTop = logContainer.scrollHeight;
        }
    }

    function applyRowFilter(row) {
        var level = row.getAttribute("data-level");
        var cap = row.getAttribute("data-cap");
        var msgCell = row.querySelector(".col-msg");
        var text = msgCell ? msgCell.textContent : "";

        var visible = levelFilters[level] &&
            (capFilter === "" || cap === capFilter) &&
            (searchText === "" || text.toLowerCase().indexOf(searchText.toLowerCase()) !== -1);

        row.style.display = visible ? "" : "none";

        // Highlight search matches
        if (msgCell && searchText && visible) {
            highlightText(msgCell, searchText);
        } else if (msgCell && !searchText) {
            // Remove highlights
            msgCell.textContent = text;
        }
    }

    function applyAllFilters() {
        var rows = logTbody.querySelectorAll("tr");
        for (var i = 0; i < rows.length; i++) {
            applyRowFilter(rows[i]);
        }
    }

    function highlightText(el, query) {
        var text = el.textContent;
        var lower = text.toLowerCase();
        var qLower = query.toLowerCase();
        var idx = lower.indexOf(qLower);
        if (idx === -1) {
            el.textContent = text;
            return;
        }
        el.innerHTML = "";
        var pos = 0;
        while (idx !== -1) {
            el.appendChild(document.createTextNode(text.substring(pos, idx)));
            var mark = document.createElement("mark");
            mark.textContent = text.substring(idx, idx + query.length);
            el.appendChild(mark);
            pos = idx + query.length;
            idx = lower.indexOf(qLower, pos);
        }
        el.appendChild(document.createTextNode(text.substring(pos)));
    }

    // Auto-scroll detection
    logContainer.addEventListener("scroll", function () {
        var atBottom = logContainer.scrollTop + logContainer.clientHeight >= logContainer.scrollHeight - 20;
        autoScroll = atBottom;
        jumpBtn.style.display = atBottom ? "none" : "block";
    });

    document.getElementById("log-jump-btn").addEventListener("click", function () {
        logContainer.scrollTop = logContainer.scrollHeight;
        autoScroll = true;
        jumpBtn.style.display = "none";
    });

    // Log search
    document.getElementById("log-search").addEventListener("input", function (e) {
        searchText = e.target.value;
        applyAllFilters();
    });

    // Capability filter
    document.getElementById("log-cap-filter").addEventListener("change", function (e) {
        capFilter = e.target.value;
        applyAllFilters();
    });

    function updateCapFilter() {
        var sel = document.getElementById("log-cap-filter");
        var current = sel.value;
        sel.innerHTML = '<option value="">All capabilities</option>';
        for (var i = 0; i < knownCapNames.length; i++) {
            var opt = document.createElement("option");
            opt.value = knownCapNames[i];
            opt.textContent = knownCapNames[i];
            sel.appendChild(opt);
        }
        sel.value = current;
    }

    // Level filter buttons
    var levelBtns = document.querySelectorAll(".xp-btn-level");
    for (var i = 0; i < levelBtns.length; i++) {
        levelBtns[i].addEventListener("click", function (e) {
            var level = e.target.getAttribute("data-level");
            levelFilters[level] = !levelFilters[level];
            e.target.classList.toggle("active", levelFilters[level]);
            applyAllFilters();
        });
    }

    // --- Historical log loading ---
    document.getElementById("log-date").addEventListener("change", function (e) {
        var date = e.target.value;
        if (!date) return;
        historicalMode = true;
        logTbody.innerHTML = "";
        fetch("/api/dashboard/logs/" + encodeURIComponent(date) + "?token=" + encodeURIComponent(TOKEN))
            .then(function (resp) {
                if (!resp.ok) return resp.text().then(function (t) { throw new Error(t); });
                return resp.json();
            })
            .then(function (entries) {
                for (var i = 0; i < entries.length; i++) {
                    appendLog(entries[i].capability, entries[i].entry);
                }
            })
            .catch(function (err) {
                console.error("Failed to load historical logs:", err);
            });
    });

    document.getElementById("log-live-btn").addEventListener("click", function () {
        historicalMode = false;
        logTbody.innerHTML = "";
        document.getElementById("log-date").value = "";
        autoScroll = true;
    });

    // =========================================================================
    // Window minimize/restore via taskbar
    // =========================================================================
    function syncTaskbarState() {
        var items = document.querySelectorAll(".taskbar-item[data-window]");
        for (var i = 0; i < items.length; i++) {
            var winId = items[i].getAttribute("data-window");
            var win = document.getElementById(winId);
            if (win) {
                items[i].classList.toggle("active", !win.classList.contains("minimized"));
            }
        }
    }

    function toggleWindow(winId) {
        var win = document.getElementById(winId);
        if (!win) return;
        win.classList.toggle("minimized");
        syncTaskbarState();
    }

    // Taskbar buttons toggle minimize
    var taskbarItems = document.querySelectorAll(".taskbar-item[data-window]");
    for (var ti = 0; ti < taskbarItems.length; ti++) {
        taskbarItems[ti].addEventListener("click", function (e) {
            toggleWindow(e.target.getAttribute("data-window"));
        });
    }

    // Titlebar minimize buttons
    var minimizeBtns = document.querySelectorAll(".xp-btn-minimize[data-window]");
    for (var mi = 0; mi < minimizeBtns.length; mi++) {
        minimizeBtns[mi].addEventListener("click", function (e) {
            toggleWindow(e.target.getAttribute("data-window"));
        });
    }

    // Titlebar close buttons (same as minimize — hide the window)
    var closeBtns = document.querySelectorAll(".xp-btn-close[data-window]");
    for (var ci = 0; ci < closeBtns.length; ci++) {
        closeBtns[ci].addEventListener("click", function (e) {
            var winId = e.target.getAttribute("data-window");
            var win = document.getElementById(winId);
            if (win && !win.classList.contains("minimized")) {
                win.classList.add("minimized");
                syncTaskbarState();
            }
        });
    }

    // =========================================================================
    // Start Menu
    // =========================================================================
    document.getElementById("start-btn").addEventListener("click", function (e) {
        e.stopPropagation();
        var menu = document.getElementById("start-menu");
        menu.style.display = menu.style.display === "none" ? "block" : "none";
    });

    // Start > Settings opens the settings window
    document.getElementById("start-settings").addEventListener("click", function (e) {
        e.preventDefault();
        var win = document.getElementById("settings-window");
        if (win.classList.contains("minimized")) {
            win.classList.remove("minimized");
            syncTaskbarState();
        }
        document.getElementById("start-menu").style.display = "none";
    });

    document.addEventListener("click", function (e) {
        var menu = document.getElementById("start-menu");
        if (!menu.contains(e.target) && e.target.id !== "start-btn") {
            menu.style.display = "none";
        }
    });

    // =========================================================================
    // Settings form
    // =========================================================================
    function loadSettingsForm(config) {
        setVal("general_data_dir", config.general.data_dir);
        setVal("general_log_level", config.general.log_level);

        setVal("llm_provider", config.llm.provider);
        setVal("llm_base_url", config.llm.base_url);
        setVal("llm_model", config.llm.model);
        setVal("llm_api_key_env", config.llm.api_key_env);
        setVal("llm_temperature", config.llm.temperature);
        setVal("llm_max_concurrent", config.llm.max_concurrent);

        setVal("news_bind", config.news.bind);
        setVal("news_title", config.news.title);
        setVal("news_editor_token", config.news.editor_token || "");

        setChecked("safety_enabled", config.safety.enabled);
        setVal("safety_max_output_bytes", config.safety.max_output_bytes);
        setChecked("safety_sanitize_injections", config.safety.sanitize_injections);
        setChecked("safety_detect_leaks", config.safety.detect_leaks);
        setChecked("safety_boundary_wrap", config.safety.boundary_wrap);
        setVal("safety_high_severity_action", config.safety.high_severity_action);

        // MCP servers
        var mcpContainer = document.getElementById("mcpServers");
        mcpContainer.innerHTML = "";
        if (config.mcp && config.mcp.servers) {
            Object.keys(config.mcp.servers).forEach(function (name) {
                var srv = config.mcp.servers[name];
                addMcpServerRow(name, srv.command, srv.args || [], srv.env || {});
            });
        }

        // Capabilities
        var capContainer = document.getElementById("capabilitiesConfig");
        capContainer.innerHTML = "";
        (config.capabilities || []).forEach(function (cap) {
            addCapabilityRow(cap);
        });
    }

    function gatherSettingsForm() {
        var config = {
            general: {
                data_dir: getVal("general_data_dir"),
                log_level: getVal("general_log_level"),
            },
            llm: {
                provider: getVal("llm_provider"),
                base_url: getVal("llm_base_url"),
                model: getVal("llm_model"),
                api_key_env: getVal("llm_api_key_env"),
                temperature: parseFloat(getVal("llm_temperature")) || 0.3,
                max_concurrent: parseInt(getVal("llm_max_concurrent"), 10) || 10,
            },
            news: {
                bind: getVal("news_bind"),
                title: getVal("news_title"),
            },
            safety: {
                enabled: getChecked("safety_enabled"),
                max_output_bytes: parseInt(getVal("safety_max_output_bytes"), 10) || 100000,
                sanitize_injections: getChecked("safety_sanitize_injections"),
                detect_leaks: getChecked("safety_detect_leaks"),
                boundary_wrap: getChecked("safety_boundary_wrap"),
                high_severity_action: getVal("safety_high_severity_action"),
                custom_block_patterns: CONFIG.safety.custom_block_patterns || [],
            },
            mcp: { servers: {} },
            capabilities: [],
        };

        var token = getVal("news_editor_token");
        if (token) config.news.editor_token = token;

        // Gather MCP servers
        document.querySelectorAll("#mcpServers .xp-dynamic-row").forEach(function (row) {
            var name = row.querySelector(".mcp-name").value.trim();
            if (!name) return;
            var command = row.querySelector(".mcp-command").value.trim();
            var argsStr = row.querySelector(".mcp-args").value.trim();
            var envStr = row.querySelector(".mcp-env").value.trim();
            var args = argsStr ? argsStr.split(",").map(function (s) { return s.trim(); }) : [];
            var env = {};
            if (envStr) {
                envStr.split(",").forEach(function (pair) {
                    var parts = pair.split("=");
                    if (parts.length === 2) env[parts[0].trim()] = parts[1].trim();
                });
            }
            config.mcp.servers[name] = { command: command, args: args, env: env };
        });

        // Gather capabilities
        document.querySelectorAll("#capabilitiesConfig .xp-dynamic-row").forEach(function (row) {
            var cap = {
                name: row.querySelector(".cap-cfg-name").value.trim(),
                description: row.querySelector(".cap-cfg-description").value.trim(),
                schedule: row.querySelector(".cap-cfg-schedule").value.trim(),
                mcp_servers: row.querySelector(".cap-cfg-mcp-servers").value.trim().split(",").map(function (s) { return s.trim(); }).filter(Boolean),
                enabled: row.querySelector(".cap-cfg-enabled").checked,
            };
            var configPath = row.querySelector(".cap-cfg-config-path").value.trim();
            if (configPath) cap.config_path = configPath;
            if (cap.name) config.capabilities.push(cap);
        });

        return config;
    }

    function addMcpServerRow(name, command, args, env) {
        var container = document.getElementById("mcpServers");
        var row = document.createElement("div");
        row.className = "xp-dynamic-row";

        var envParts = [];
        if (env) {
            Object.keys(env).forEach(function (k) { envParts.push(k + "=" + env[k]); });
        }

        row.innerHTML =
            '<div class="xp-fields-grid">' +
            '<div class="xp-field"><label>Name</label><input type="text" class="xp-input xp-input-wide mcp-name" value="' + esc(name || "") + '"></div>' +
            '<div class="xp-field"><label>Command</label><input type="text" class="xp-input xp-input-wide mcp-command" value="' + esc(command || "") + '"></div>' +
            '<div class="xp-field"><label>Args (comma-sep)</label><input type="text" class="xp-input xp-input-wide mcp-args" value="' + esc((args || []).join(", ")) + '"></div>' +
            '<div class="xp-field"><label>Env (K=V, ...)</label><input type="text" class="xp-input xp-input-wide mcp-env" value="' + esc(envParts.join(", ")) + '"></div>' +
            '</div>' +
            '<button type="button" class="xp-button xp-button-remove">Remove</button>';

        row.querySelector(".xp-button-remove").addEventListener("click", function () { row.remove(); });
        container.appendChild(row);
    }

    function addCapabilityRow(cap) {
        var container = document.getElementById("capabilitiesConfig");
        var row = document.createElement("div");
        row.className = "xp-dynamic-row";
        cap = cap || {};

        row.innerHTML =
            '<div class="xp-fields-grid">' +
            '<div class="xp-field"><label>Name</label><input type="text" class="xp-input xp-input-wide cap-cfg-name" value="' + esc(cap.name || "") + '"></div>' +
            '<div class="xp-field"><label>Description</label><input type="text" class="xp-input xp-input-wide cap-cfg-description" value="' + esc(cap.description || "") + '"></div>' +
            '<div class="xp-field"><label>Schedule (cron)</label><input type="text" class="xp-input xp-input-wide cap-cfg-schedule" value="' + esc(cap.schedule || "") + '"></div>' +
            '<div class="xp-field"><label>MCP servers (comma-sep)</label><input type="text" class="xp-input xp-input-wide cap-cfg-mcp-servers" value="' + esc((cap.mcp_servers || []).join(", ")) + '"></div>' +
            '<div class="xp-field"><label>Config path</label><input type="text" class="xp-input xp-input-wide cap-cfg-config-path" value="' + esc(cap.config_path || "") + '"></div>' +
            '<div class="xp-field xp-field-checkbox"><label><input type="checkbox" class="cap-cfg-enabled"' + (cap.enabled !== false ? " checked" : "") + '> Enabled</label></div>' +
            '</div>' +
            '<button type="button" class="xp-button xp-button-remove">Remove</button>';

        row.querySelector(".xp-button-remove").addEventListener("click", function () { row.remove(); });
        container.appendChild(row);
    }

    function saveSettings() {
        var config = gatherSettingsForm();
        var status = document.getElementById("saveStatus");
        status.textContent = "Saving...";
        status.className = "save-status";

        fetch("/api/settings", {
            method: "PUT",
            headers: {
                "Authorization": "Bearer " + TOKEN,
                "Content-Type": "application/json",
            },
            body: JSON.stringify(config),
        }).then(function (resp) {
            if (resp.ok) {
                status.textContent = "Saved!";
                status.className = "save-status save-ok";
                // Update local CONFIG reference
                CONFIG = config;
                // If token changed, update our working token
                if (config.news.editor_token) {
                    TOKEN = config.news.editor_token;
                    window.__DASHBOARD_TOKEN = TOKEN;
                }
            } else {
                return resp.text().then(function (t) {
                    status.textContent = "Error: " + t;
                    status.className = "save-status save-err";
                });
            }
        }).catch(function (err) {
            status.textContent = "Network error: " + err;
            status.className = "save-status save-err";
        });
    }

    document.getElementById("settingsForm").addEventListener("submit", function (e) {
        e.preventDefault();
        saveSettings();
    });

    document.getElementById("add-mcp-btn").addEventListener("click", function () {
        addMcpServerRow("", "", [], {});
    });

    document.getElementById("add-cap-btn").addEventListener("click", function () {
        addCapabilityRow({});
    });

    // Load settings form with initial config
    loadSettingsForm(CONFIG);

    // =========================================================================
    // Clock
    // =========================================================================
    function updateClock() {
        var now = new Date();
        var h = now.getHours().toString().padStart(2, "0");
        var m = now.getMinutes().toString().padStart(2, "0");
        document.getElementById("tray-clock").textContent = h + ":" + m;
    }
    setInterval(updateClock, 10000);
    updateClock();

    // =========================================================================
    // Utility
    // =========================================================================
    function setVal(id, value) {
        var el = document.getElementById(id);
        if (el) el.value = value !== undefined && value !== null ? value : "";
    }

    function setChecked(id, value) {
        var el = document.getElementById(id);
        if (el) el.checked = !!value;
    }

    function getVal(id) {
        var el = document.getElementById(id);
        return el ? el.value : "";
    }

    function getChecked(id) {
        var el = document.getElementById(id);
        return el ? el.checked : false;
    }

    function formatTime(isoStr) {
        try {
            var d = new Date(isoStr);
            return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
        } catch (_) {
            return isoStr;
        }
    }

    function formatTimestamp(isoStr) {
        try {
            var d = new Date(isoStr);
            var h = d.getHours().toString().padStart(2, "0");
            var m = d.getMinutes().toString().padStart(2, "0");
            var s = d.getSeconds().toString().padStart(2, "0");
            var ms = d.getMilliseconds().toString().padStart(3, "0");
            return h + ":" + m + ":" + s + "." + ms;
        } catch (_) {
            return isoStr;
        }
    }

    function formatDuration(totalSecs) {
        var days = Math.floor(totalSecs / 86400);
        var hours = Math.floor((totalSecs % 86400) / 3600);
        var mins = Math.floor((totalSecs % 3600) / 60);
        var secs = totalSecs % 60;
        if (days > 0) return days + "d " + hours + "h " + mins + "m";
        if (hours > 0) return hours + "h " + mins + "m " + secs + "s";
        if (mins > 0) return mins + "m " + secs + "s";
        return secs + "s";
    }

    function escapeHtml(s) {
        var div = document.createElement("div");
        div.textContent = s;
        return div.innerHTML;
    }

    function escapeAttr(s) {
        return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    }

    function esc(str) {
        var div = document.createElement("div");
        div.textContent = str;
        return div.innerHTML.replace(/"/g, "&quot;");
    }

    // --- Init ---
    syncTaskbarState();
    connect();
})();
