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
    var topZIndex = 10;

    function bringToFront(win) {
        if (!win || win.classList.contains("minimized")) return;
        topZIndex++;
        if (topZIndex >= 9000) {
            // Reset all window z-indices to avoid colliding with taskbar (9999)
            var allWins = document.querySelectorAll(".xp-window");
            var ordered = Array.prototype.slice.call(allWins).sort(function (a, b) {
                return (parseInt(a.style.zIndex) || 0) - (parseInt(b.style.zIndex) || 0);
            });
            for (var i = 0; i < ordered.length; i++) {
                ordered[i].style.zIndex = 10 + i;
            }
            topZIndex = 10 + ordered.length;
        }
        win.style.zIndex = topZIndex;
    }

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
        if (win.classList.contains("minimized")) {
            win.classList.remove("minimized");
            bringToFront(win);
        } else {
            win.classList.add("minimized");
        }
        syncTaskbarState();
    }

    // Click anywhere on a window to bring it to front
    var allWindows = document.querySelectorAll(".xp-window");
    for (var wi = 0; wi < allWindows.length; wi++) {
        allWindows[wi].addEventListener("mousedown", function (e) {
            bringToFront(this);
        });
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
        openWindow("settings-window");
        document.getElementById("start-menu").style.display = "none";
    });

    // Start > MCP Store
    document.getElementById("start-mcp-store").addEventListener("click", function (e) {
        e.preventDefault();
        openWindow("mcp-store-window");
        loadUnifiedStore();
        document.getElementById("start-menu").style.display = "none";
    });

    function openWindow(winId) {
        var win = document.getElementById(winId);
        if (!win) return;
        if (win.classList.contains("minimized")) {
            win.classList.remove("minimized");
            syncTaskbarState();
        }
        bringToFront(win);
    }

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
        setVal("llm_api_key", config.llm.api_key);
        setVal("llm_temperature", config.llm.temperature);
        setVal("llm_max_concurrent", config.llm.max_concurrent);
        setVal("llm_extra_body", jsonPretty(config.llm.extra_body));

        setVal("news_bind", config.news.bind);
        setVal("news_title", config.news.title);
        setVal("news_editor_token", config.news.editor_token || "");

        setChecked("safety_enabled", config.safety.enabled);
        setVal("safety_max_output_bytes", config.safety.max_output_bytes);
        setChecked("safety_sanitize_injections", config.safety.sanitize_injections);
        setChecked("safety_detect_leaks", config.safety.detect_leaks);
        setChecked("safety_boundary_wrap", config.safety.boundary_wrap);
        setVal("safety_high_severity_action", config.safety.high_severity_action);

        // Capabilities
        var capContainer = document.getElementById("capabilitiesConfig");
        capContainer.innerHTML = "";
        (config.capabilities || []).forEach(function (cap) {
            addCapabilityRow(cap);
        });
    }

    function gatherSettingsForm() {
        var globalExtra = tryParseJson("llm_extra_body");
        if (globalExtra === null) return null; // validation failed

        var config = {
            general: {
                data_dir: getVal("general_data_dir"),
                log_level: getVal("general_log_level"),
            },
            llm: {
                provider: getVal("llm_provider"),
                base_url: getVal("llm_base_url"),
                model: getVal("llm_model"),
                api_key: getVal("llm_api_key"),
                temperature: parseFloat(getVal("llm_temperature")) || 0.3,
                max_concurrent: parseInt(getVal("llm_max_concurrent"), 10) || 10,
                extra_body: globalExtra,
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

        // Gather capabilities
        var capValid = true;
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

            // Restore plugin and log_level from data attributes
            var plugin = row.dataset.plugin;
            if (plugin) cap.plugin = plugin;
            var logLevel = row.dataset.logLevel;
            if (logLevel) cap.log_level = logLevel;

            // Gather LLM overrides
            var llmOverrides = gatherCapLlmOverrides(row);
            if (llmOverrides === false) {
                capValid = false;
            } else if (llmOverrides) {
                cap.llm = llmOverrides;
            }

            if (cap.name) config.capabilities.push(cap);
        });

        if (!capValid) return null;
        return config;
    }

    function gatherCapLlmOverrides(row) {
        var model = (row.querySelector(".cap-llm-model") || {}).value || "";
        var temp = (row.querySelector(".cap-llm-temperature") || {}).value || "";
        var maxTokens = (row.querySelector(".cap-llm-max-tokens") || {}).value || "";
        var maxConc = (row.querySelector(".cap-llm-max-concurrent") || {}).value || "";
        var extraEl = row.querySelector(".cap-llm-extra-body");
        var extraStr = extraEl ? extraEl.value.trim() : "";
        var errEl = row.querySelector(".cap-llm-extra-body-err");

        model = model.trim();
        temp = temp.trim();
        maxTokens = maxTokens.trim();
        maxConc = maxConc.trim();

        var hasAny = model || temp || maxTokens || maxConc || (extraStr && extraStr !== "{}");
        if (!hasAny) return null; // no overrides

        var llm = {};
        if (model) llm.model = model;
        if (temp) llm.temperature = parseFloat(temp);
        if (maxTokens) llm.max_completion_tokens = parseInt(maxTokens, 10);
        if (maxConc) llm.max_concurrent = parseInt(maxConc, 10);

        if (extraStr && extraStr !== "{}") {
            try {
                llm.extra_body = JSON.parse(extraStr);
                if (errEl) { errEl.textContent = ""; }
            } catch (e) {
                if (errEl) { errEl.textContent = "Invalid JSON: " + e.message; }
                return false; // signal validation failure
            }
        }

        return Object.keys(llm).length > 0 ? llm : null;
    }

    function addCapabilityRow(cap) {
        var container = document.getElementById("capabilitiesConfig");
        var row = document.createElement("div");
        row.className = "xp-dynamic-row";
        cap = cap || {};

        // Store plugin and log_level as data attributes so they survive round-trip
        if (cap.plugin) row.dataset.plugin = cap.plugin;
        if (cap.log_level) row.dataset.logLevel = cap.log_level;

        var llm = cap.llm || {};
        var llmExtraStr = jsonPretty(llm.extra_body);

        row.innerHTML =
            '<div class="xp-fields-grid">' +
            '<div class="xp-field"><label>Name</label><input type="text" class="xp-input xp-input-wide cap-cfg-name" value="' + esc(cap.name || "") + '"></div>' +
            '<div class="xp-field"><label>Description</label><input type="text" class="xp-input xp-input-wide cap-cfg-description" value="' + esc(cap.description || "") + '"></div>' +
            '<div class="xp-field"><label>Schedule (cron)</label><input type="text" class="xp-input xp-input-wide cap-cfg-schedule" value="' + esc(cap.schedule || "") + '"></div>' +
            '<div class="xp-field"><label>MCP servers (comma-sep)</label><input type="text" class="xp-input xp-input-wide cap-cfg-mcp-servers" value="' + esc((cap.mcp_servers || []).join(", ")) + '"></div>' +
            '<div class="xp-field"><label>Config path</label><input type="text" class="xp-input xp-input-wide cap-cfg-config-path" value="' + esc(cap.config_path || "") + '"></div>' +
            '<div class="xp-field xp-field-checkbox"><label><input type="checkbox" class="cap-cfg-enabled"' + (cap.enabled !== false ? " checked" : "") + '> Enabled</label></div>' +
            '<div class="cap-llm-overrides">' +
                '<button type="button" class="cap-llm-toggle">LLM overrides &#9656;</button>' +
                '<div class="cap-llm-body">' +
                    '<div class="xp-field"><label>Model</label><input type="text" class="xp-input xp-input-wide cap-llm-model" value="' + esc(llm.model || "") + '"></div>' +
                    '<div class="xp-field"><label>Temperature</label><input type="number" class="xp-input cap-llm-temperature" step="0.1" min="0" max="2" style="width:80px;" value="' + esc(llm.temperature != null ? String(llm.temperature) : "") + '"></div>' +
                    '<div class="xp-field"><label>Max tokens</label><input type="number" class="xp-input cap-llm-max-tokens" min="1" style="width:100px;" value="' + esc(llm.max_completion_tokens != null ? String(llm.max_completion_tokens) : "") + '"></div>' +
                    '<div class="xp-field"><label>Max concurrent</label><input type="number" class="xp-input cap-llm-max-concurrent" min="1" style="width:80px;" value="' + esc(llm.max_concurrent != null ? String(llm.max_concurrent) : "") + '"></div>' +
                    '<div class="xp-field xp-field-full"><label>Extra body (JSON)</label>' +
                        '<textarea class="xp-input xp-textarea cap-llm-extra-body" rows="2" placeholder=\'{"stream": false}\'>' + esc(llmExtraStr) + '</textarea>' +
                        '<span class="xp-field-error cap-llm-extra-body-err"></span>' +
                    '</div>' +
                '</div>' +
            '</div>' +
            '</div>' +
            '<button type="button" class="xp-button xp-button-remove">Remove</button>';

        row.querySelector(".xp-button-remove").addEventListener("click", function () { row.remove(); });
        row.querySelector(".cap-llm-toggle").addEventListener("click", function () {
            var body = row.querySelector(".cap-llm-body");
            var open = body.classList.toggle("open");
            this.innerHTML = "LLM overrides " + (open ? "&#9662;" : "&#9656;");
        });
        // Auto-expand if overrides are set
        if (llm.model || llm.temperature != null || llm.max_completion_tokens != null || llm.max_concurrent != null || (llm.extra_body && Object.keys(llm.extra_body).length > 0)) {
            var body = row.querySelector(".cap-llm-body");
            body.classList.add("open");
            row.querySelector(".cap-llm-toggle").innerHTML = "LLM overrides &#9662;";
        }
        container.appendChild(row);
    }

    function saveSettings() {
        var config = gatherSettingsForm();
        var status = document.getElementById("saveStatus");
        if (config === null) {
            status.textContent = "Fix validation errors before saving.";
            status.className = "save-status save-err";
            return;
        }
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

    document.getElementById("add-cap-btn").addEventListener("click", function () {
        addCapabilityRow({});
    });

    // Load settings form with initial config
    loadSettingsForm(CONFIG);

    // =========================================================================
    // Schema-driven config editor (inline XP window)
    // =========================================================================
    var cfgEditorCapName = null;
    var cfgEditorSchema = null;

    function cfgEsc(str) {
        var div = document.createElement("div");
        div.textContent = str;
        return div.innerHTML.replace(/"/g, "&quot;");
    }

    // --- Schema form builder ---------------------------------------------------

    /** Build a form field element for a single ConfigField descriptor. */
    function cfgBuildField(field, value) {
        var wrapper = document.createElement("div");
        wrapper.className = "xp-field";
        wrapper.setAttribute("data-cfg-key", field.key);
        var label = field.label || field.key;

        if (field.type === "list") {
            return cfgBuildListField(field, value);
        }

        var labelEl = document.createElement("label");
        labelEl.textContent = label;
        wrapper.appendChild(labelEl);

        if (field.type === "text") {
            var inp = document.createElement("input");
            inp.type = "text";
            inp.className = "xp-input xp-input-wide";
            inp.value = value != null ? String(value) : (field.default || "");
            wrapper.appendChild(inp);
        } else if (field.type === "textarea") {
            var ta = document.createElement("textarea");
            ta.className = "xp-input xp-input-wide";
            ta.rows = 2;
            ta.value = value != null ? String(value) : (field.default || "");
            wrapper.appendChild(ta);
        } else if (field.type === "select") {
            var sel = document.createElement("select");
            sel.className = "xp-select";
            (field.options || []).forEach(function(opt) {
                var o = document.createElement("option");
                o.value = opt;
                o.textContent = opt;
                if (String(value || field.default || "") === opt) o.selected = true;
                sel.appendChild(o);
            });
            wrapper.appendChild(sel);
        } else if (field.type === "text-list") {
            var inp2 = document.createElement("input");
            inp2.type = "text";
            inp2.className = "xp-input xp-input-wide";
            inp2.value = Array.isArray(value) ? value.join(", ") : (value != null ? String(value) : "");
            wrapper.appendChild(inp2);
        }

        return wrapper;
    }

    /** Build a repeatable list field with Add/Remove buttons. */
    function cfgBuildListField(field, items) {
        items = Array.isArray(items) ? items : [];
        var container = document.createElement("div");
        container.className = "xp-fieldset cfg-list";
        container.setAttribute("data-cfg-key", field.key);

        var legend = document.createElement("div");
        legend.className = "xp-fieldset-legend";
        legend.textContent = field.label || field.key;
        container.appendChild(legend);

        var itemsContainer = document.createElement("div");
        itemsContainer.className = "cfg-list-items";
        container.appendChild(itemsContainer);

        items.forEach(function(item) { cfgAddListItem(itemsContainer, field.fields || [], item); });

        var actions = document.createElement("div");
        actions.style.marginTop = "4px";
        var addBtn = document.createElement("button");
        addBtn.type = "button";
        addBtn.className = "xp-button";
        addBtn.textContent = "+ Add " + (field.label || field.key).replace(/s$/, "").toLowerCase();
        addBtn.addEventListener("click", function() { cfgAddListItem(itemsContainer, field.fields || [], {}); });
        actions.appendChild(addBtn);
        container.appendChild(actions);

        return container;
    }

    /** Add a single item row to a list field. */
    function cfgAddListItem(container, subFields, data) {
        data = data || {};
        var row = document.createElement("div");
        row.className = "xp-dynamic-row cfg-list-item";
        row.style.marginBottom = "8px";
        row.style.padding = "6px";
        row.style.border = "1px solid #ACA899";

        var grid = document.createElement("div");
        grid.className = "xp-fields-grid";
        subFields.forEach(function(sf) {
            grid.appendChild(cfgBuildField(sf, data[sf.key]));
        });
        row.appendChild(grid);

        var rmBtn = document.createElement("button");
        rmBtn.type = "button";
        rmBtn.className = "xp-button xp-button-remove";
        rmBtn.style.marginTop = "4px";
        rmBtn.textContent = "Remove";
        rmBtn.addEventListener("click", function() { row.remove(); });
        row.appendChild(rmBtn);

        container.appendChild(row);
    }

    // --- Gather form values back into an object --------------------------------

    /** Gather the value of a single field element. */
    function cfgGatherField(el, field) {
        if (field.type === "list") {
            return cfgGatherList(el, field);
        }
        if (field.type === "text-list") {
            var raw = (el.querySelector("input") || {}).value || "";
            return raw ? raw.split(",").map(function(s) { return s.trim(); }).filter(Boolean) : [];
        }
        if (field.type === "select") {
            return (el.querySelector("select") || {}).value || "";
        }
        if (field.type === "textarea") {
            return (el.querySelector("textarea") || {}).value || "";
        }
        // text
        return (el.querySelector("input") || {}).value || "";
    }

    /** Gather all items from a list field. */
    function cfgGatherList(container, field) {
        var items = [];
        container.querySelectorAll(":scope > .cfg-list-items > .cfg-list-item").forEach(function(row) {
            var obj = {};
            (field.fields || []).forEach(function(sf) {
                var child = row.querySelector('[data-cfg-key="' + sf.key + '"]');
                if (child) obj[sf.key] = cfgGatherField(child, sf);
            });
            items.push(obj);
        });
        return items;
    }

    /** Gather the entire form into a data object, respecting root_key. */
    function cfgGatherForm(schema) {
        var container = document.getElementById("config-editor-fields");
        var data = {};
        (schema.fields || []).forEach(function(field) {
            var el = container.querySelector(':scope > [data-cfg-key="' + field.key + '"]');
            if (el) data[field.key] = cfgGatherField(el, field);
        });
        if (schema.root_key) {
            var wrapped = {};
            wrapped[schema.root_key] = data;
            return wrapped;
        }
        return data;
    }

    // --- Load / save -----------------------------------------------------------

    function cfgLoadForm(schema, data) {
        var container = document.getElementById("config-editor-fields");
        container.innerHTML = "";
        var inner = data;
        if (schema.root_key && data && typeof data === "object") {
            inner = data[schema.root_key] || {};
        }
        (schema.fields || []).forEach(function(field) {
            container.appendChild(cfgBuildField(field, inner[field.key]));
        });
    }

    function cfgSave() {
        if (!cfgEditorCapName || !cfgEditorSchema) return;
        var data = cfgGatherForm(cfgEditorSchema);
        var yaml = jsyaml.dump(data, { lineWidth: -1 });
        var status = document.getElementById("config-editor-status");
        status.textContent = "Saving...";
        status.className = "save-status";
        var xhr = new XMLHttpRequest();
        xhr.open("PUT", "/api/config/" + encodeURIComponent(cfgEditorCapName) + "?token=" + encodeURIComponent(TOKEN));
        xhr.setRequestHeader("Content-Type", "text/plain");
        xhr.onload = function() {
            if (xhr.status === 200) {
                status.textContent = "Saved.";
                status.className = "save-status save-ok";
            } else {
                status.textContent = "Error: " + xhr.responseText;
                status.className = "save-status save-err";
            }
        };
        xhr.onerror = function() {
            status.textContent = "Network error.";
            status.className = "save-status save-err";
        };
        xhr.send(yaml);
    }

    function cfgLoad(capName, schema) {
        cfgEditorCapName = capName;
        cfgEditorSchema = schema;
        var titleEl = document.getElementById("config-editor-window-title");
        titleEl.textContent = "Config Editor \u2014 " + capName;
        var xhr = new XMLHttpRequest();
        xhr.open("GET", "/api/config/" + encodeURIComponent(capName) + "?token=" + encodeURIComponent(TOKEN));
        xhr.onload = function() {
            var data = {};
            if (xhr.status === 200 && xhr.responseText.trim()) {
                try { data = jsyaml.load(xhr.responseText); } catch (e) { data = {}; }
            }
            if (!data || typeof data !== "object") data = {};
            cfgLoadForm(schema, data);
        };
        xhr.onerror = function() {
            cfgLoadForm(schema, {});
        };
        xhr.send();
    }

    document.getElementById("config-editor-save").addEventListener("click", cfgSave);

    window.__openConfigEditor = function(capName, schema) {
        cfgLoad(capName, schema);
        openWindow("config-editor-window");
    };

    // =========================================================================
    // Unified Store (table layout) — MCP Servers + Capabilities tabs
    // =========================================================================
    var storeTbody = document.getElementById("store-tbody");
    var capStoreTbody = document.getElementById("cap-store-tbody");
    var storeCatalog = null;        // cached RegistryManifest
    var installedMcps = [];         // array from /api/mcp/installed
    var installedCaps = [];         // array from /api/capabilities/installed
    var storeSearchText = "";
    var mcpTestResults = {};        // name -> { ok, tools, error }
    var activeStoreTab = "mcp";

    // Tab switching
    document.querySelectorAll(".store-tab").forEach(function (tab) {
        tab.addEventListener("click", function () {
            document.querySelectorAll(".store-tab").forEach(function (t) { t.classList.remove("active"); });
            tab.classList.add("active");
            activeStoreTab = tab.getAttribute("data-tab");
            document.getElementById("store-tab-mcp").style.display = activeStoreTab === "mcp" ? "" : "none";
            document.getElementById("store-tab-capabilities").style.display = activeStoreTab === "capabilities" ? "" : "none";
        });
    });

    function loadUnifiedStore() {
        storeTbody.innerHTML = '<tr><td colspan="7" class="loading-cell">Loading...</td></tr>';
        capStoreTbody.innerHTML = '<tr><td colspan="6" class="loading-cell">Loading...</td></tr>';

        Promise.all([
            apiFetch("/api/registry/catalog"),
            apiFetch("/api/mcp/installed"),
            apiFetch("/api/capabilities/installed"),
        ]).then(function (results) {
            storeCatalog = results[0];
            installedMcps = results[1];
            installedCaps = results[2];
            renderStoreTable();
            renderCapStoreTable();
        }).catch(function (err) {
            storeTbody.innerHTML = '<tr><td colspan="7" class="loading-cell">Failed to load: ' +
                escapeHtml(String(err)) + '</td></tr>';
            capStoreTbody.innerHTML = '<tr><td colspan="6" class="loading-cell">Failed to load: ' +
                escapeHtml(String(err)) + '</td></tr>';
        });
    }

    function renderStoreTable() {
        if (!storeCatalog) return;
        var servers = storeCatalog.servers || [];
        var filtered = servers;

        if (storeSearchText) {
            var q = storeSearchText.toLowerCase();
            filtered = servers.filter(function (s) {
                return s.name.toLowerCase().indexOf(q) !== -1 ||
                    s.description.toLowerCase().indexOf(q) !== -1 ||
                    (s.tags || []).some(function (t) { return t.toLowerCase().indexOf(q) !== -1; });
            });
        }

        if (filtered.length === 0) {
            storeTbody.innerHTML = '<tr><td colspan="7" class="loading-cell">No servers found</td></tr>';
            return;
        }

        // Build installed lookup by registry_id or name
        var installedIds = {};
        installedMcps.forEach(function (m) {
            installedIds[m.registry_id || m.name] = true;
            installedIds[m.name] = true;
        });

        var html = "";
        for (var i = 0; i < filtered.length; i++) {
            var s = filtered[i];
            var isInstalled = installedIds[s.id] || false;
            var dotHtml;
            if (isInstalled) {
                dotHtml = '<span class="mcp-installed-dot active store-test-dot" data-name="' +
                    escapeAttr(s.id) + '" title="Test: list tools"></span>';
            } else {
                dotHtml = '<span class="mcp-installed-dot"></span>';
            }
            var verifiedHtml = s.verified ? '<span class="store-verified-badge" title="Verified">&#10003;</span>' : '';

            var configHtml = isInstalled
                ? '<button class="xp-button-icon store-configure-btn" data-name="' + escapeAttr(s.id) + '" title="Configure">&#9881;</button>'
                : '';

            var actionHtml;
            if (isInstalled) {
                actionHtml = '<button class="xp-button xp-button-remove store-remove-btn" data-name="' +
                    escapeAttr(s.id) + '">Remove</button>';
            } else {
                actionHtml = '<button class="xp-button xp-button-primary store-install-btn" data-id="' +
                    escapeAttr(s.id) + '">Install</button>';
            }

            html += '<tr>' +
                '<td style="text-align:center;">' + dotHtml + '</td>' +
                '<td><span class="cap-name">' + escapeHtml(s.name) + '</span></td>' +
                '<td class="store-desc-cell" title="' + escapeAttr(s.description) + '">' + escapeHtml(s.description) + '</td>' +
                '<td>v' + escapeHtml(s.version) + '</td>' +
                '<td style="text-align:center;">' + verifiedHtml + '</td>' +
                '<td style="text-align:center;">' + configHtml + '</td>' +
                '<td>' + actionHtml + '</td>' +
                '</tr>';
        }
        storeTbody.innerHTML = html;

        // Bind install buttons
        storeTbody.querySelectorAll(".store-install-btn").forEach(function (btn) {
            btn.addEventListener("click", function () {
                var id = btn.getAttribute("data-id");
                var entry = storeCatalog.servers.find(function (s) { return s.id === id; });
                if (entry) openInstallModal(entry);
            });
        });

        // Bind remove buttons
        storeTbody.querySelectorAll(".store-remove-btn").forEach(function (btn) {
            btn.addEventListener("click", function () {
                var name = btn.getAttribute("data-name");
                onMcpRemove(name, btn);
            });
        });

        // Bind configure buttons
        storeTbody.querySelectorAll(".store-configure-btn").forEach(function (btn) {
            btn.addEventListener("click", function () {
                var name = btn.getAttribute("data-name");
                openConfigureModal(name);
            });
        });

        // Bind installed-dot test buttons
        storeTbody.querySelectorAll(".store-test-dot").forEach(function (dot) {
            dot.addEventListener("click", function () {
                var name = dot.getAttribute("data-name");
                onDotTest(name);
            });
        });
    }

    function onMcpRemove(name, btn) {
        if (!confirm('Remove MCP server "' + name + '"?')) return;
        if (btn) btn.disabled = true;
        apiFetch("/api/mcp/uninstall/" + encodeURIComponent(name), { method: "POST" })
            .then(function () {
                loadUnifiedStore();
            })
            .catch(function (err) {
                alert("Uninstall failed: " + err);
                if (btn) btn.disabled = false;
            });
    }

    function onDotTest(name) {
        openActionModal("Test: " + name);
        updateActionModal(renderStepHtml([{ label: "Running list_tools on " + name + "...", state: "active" }]));

        apiFetch("/api/mcp/test/" + encodeURIComponent(name), { method: "POST" })
            .then(function (result) {
                if (result.ok) {
                    updateActionModal(
                        renderStepHtml([{ label: "Test complete", state: "done" }]) +
                        renderToolList(result.tools)
                    );
                } else {
                    updateActionModal(
                        renderStepHtml([{ label: "Test failed", state: "error" }]) +
                        '<div class="action-modal-error">' + escapeHtml(result.error) + '</div>'
                    );
                }
            })
            .catch(function (err) {
                updateActionModal(
                    renderStepHtml([{ label: "Test failed", state: "error" }]) +
                    '<div class="action-modal-error">' + escapeHtml(String(err)) + '</div>'
                );
            });
    }

    // ── Capabilities Store Tab ──────────────────────────────────────────────

    function renderCapStoreTable() {
        if (!storeCatalog) return;
        var caps = storeCatalog.capabilities || [];
        var filtered = caps;

        if (storeSearchText) {
            var q = storeSearchText.toLowerCase();
            filtered = caps.filter(function (c) {
                return c.name.toLowerCase().indexOf(q) !== -1 ||
                    c.description.toLowerCase().indexOf(q) !== -1 ||
                    (c.tags || []).some(function (t) { return t.toLowerCase().indexOf(q) !== -1; });
            });
        }

        if (filtered.length === 0) {
            capStoreTbody.innerHTML = '<tr><td colspan="6" class="loading-cell">No capabilities found</td></tr>';
            return;
        }

        // Build installed lookup
        var installedNames = {};
        installedCaps.forEach(function (c) { installedNames[c.name] = true; });

        var html = "";
        for (var i = 0; i < filtered.length; i++) {
            var c = filtered[i];
            var isInstalled = installedNames[c.id] || false;
            var dotHtml = isInstalled
                ? '<span class="mcp-installed-dot active"></span>'
                : '<span class="mcp-installed-dot"></span>';
            var verifiedHtml = c.verified ? '<span class="store-verified-badge" title="Verified">&#10003;</span>' : '';

            var actionHtml;
            if (isInstalled) {
                actionHtml = '<button class="xp-button xp-button-remove cap-store-remove-btn" data-name="' +
                    escapeAttr(c.id) + '">Remove</button>';
            } else {
                actionHtml = '<button class="xp-button xp-button-primary cap-store-install-btn" data-id="' +
                    escapeAttr(c.id) + '">Install</button>';
            }

            html += '<tr>' +
                '<td style="text-align:center;">' + dotHtml + '</td>' +
                '<td><span class="cap-name">' + escapeHtml(c.name) + '</span></td>' +
                '<td class="store-desc-cell" title="' + escapeAttr(c.description) + '">' + escapeHtml(c.description) + '</td>' +
                '<td>v' + escapeHtml(c.version) + '</td>' +
                '<td style="text-align:center;">' + verifiedHtml + '</td>' +
                '<td>' + actionHtml + '</td>' +
                '</tr>';
        }
        capStoreTbody.innerHTML = html;

        // Bind install buttons
        capStoreTbody.querySelectorAll(".cap-store-install-btn").forEach(function (btn) {
            btn.addEventListener("click", function () {
                var id = btn.getAttribute("data-id");
                onCapInstall(id, btn);
            });
        });

        // Bind remove buttons
        capStoreTbody.querySelectorAll(".cap-store-remove-btn").forEach(function (btn) {
            btn.addEventListener("click", function () {
                var name = btn.getAttribute("data-name");
                onCapRemove(name, btn);
            });
        });
    }

    function showRestartBanner() {
        document.getElementById("store-restart-banner").style.display = "";
    }

    function triggerRestart() {
        showRestartBanner();
        apiFetch("/api/system/restart", { method: "POST" }).catch(function () {
            // Connection will drop when the process exits — that's expected.
        });
        // Poll until the server is back, then reload the page.
        setTimeout(function poll() {
            fetch("/api/dashboard/status?token=" + encodeURIComponent(TOKEN))
                .then(function (r) { if (r.ok) location.reload(); else setTimeout(poll, 1000); })
                .catch(function () { setTimeout(poll, 1000); });
        }, 2000);
    }

    function onCapInstall(id, btn) {
        btn.disabled = true;
        btn.textContent = "Installing...";
        apiFetch("/api/capabilities/install", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ id: id }),
        }).then(function (result) {
            triggerRestart();
        }).catch(function (err) {
            alert("Install failed: " + err);
            btn.disabled = false;
            btn.textContent = "Install";
        });
    }

    function onCapRemove(name, btn) {
        if (!confirm('Remove capability "' + name + '"?')) return;
        btn.disabled = true;
        apiFetch("/api/capabilities/uninstall/" + encodeURIComponent(name), { method: "POST" })
            .then(function () {
                triggerRestart();
            })
            .catch(function (err) {
                alert("Uninstall failed: " + err);
                btn.disabled = false;
            });
    }

    // Store search
    document.getElementById("store-search").addEventListener("input", function (e) {
        storeSearchText = e.target.value;
        renderStoreTable();
        renderCapStoreTable();
    });

    // Store refresh
    document.getElementById("store-refresh-btn").addEventListener("click", function () {
        apiFetch("/api/registry/refresh", { method: "POST" })
            .then(function () { loadUnifiedStore(); })
            .catch(function (err) { alert("Refresh failed: " + err); });
    });

    // Load unified store on init
    loadUnifiedStore();

    // =========================================================================
    // Install Modal
    // =========================================================================
    var installModal = document.getElementById("install-modal");
    var currentInstallEntry = null;

    function renderInstallEnvVars(container, envSpecs, existingEnv) {
        var html = '<div style="font-size:11px;font-weight:bold;margin-bottom:4px;">Environment variables:</div>';
        envSpecs.forEach(function (spec) {
            var val = existingEnv[spec.name] || spec.name;
            html += '<div class="xp-field">' +
                '<label>' + escapeHtml(spec.name) + (spec.required ? ' *' : '') + '</label>' +
                '<input type="text" class="xp-input xp-input-wide install-env-input" ' +
                'data-env-name="' + escapeAttr(spec.name) + '" ' +
                'value="' + escapeAttr(val) + '" ' +
                'placeholder="env var name for ' + escapeAttr(spec.name) + '">' +
                '<div class="env-desc">' + escapeHtml(spec.description) + '</div>' +
                '</div>';
        });
        container.innerHTML = html;
    }

    function openInstallModal(entry) {
        currentInstallEntry = entry;
        document.getElementById("install-modal-title").textContent = "Install " + entry.name;
        document.getElementById("install-modal-status").textContent = "";

        // Show dependency status
        var depsDiv = document.getElementById("install-modal-deps");
        if (entry.dependencies && entry.dependencies.length > 0) {
            depsDiv.innerHTML = '<div style="font-size:11px;font-weight:bold;margin-bottom:4px;">Dependencies:</div>';
            depsDiv.innerHTML += entry.dependencies.map(function (d) {
                return '<div class="install-dep-row"><span>Checking ' + escapeHtml(d) + '...</span></div>';
            }).join("");

            // Check deps async
            apiFetch("/api/mcp/deps/" + encodeURIComponent(entry.id))
                .then(function (results) {
                    var html = '<div style="font-size:11px;font-weight:bold;margin-bottom:4px;">Dependencies:</div>';
                    Object.keys(results).forEach(function (dep) {
                        var status = results[dep];
                        var cls = status.found ? "dep-ok" : "dep-missing";
                        var icon = status.found ? "&#10003;" : "&#10007;";
                        var ver = status.version ? " (" + escapeHtml(status.version) + ")" : "";
                        html += '<div class="install-dep-row"><span class="' + cls + '">' + icon + '</span> ' +
                            escapeHtml(dep) + ver + '</div>';
                    });
                    depsDiv.innerHTML = html;
                })
                .catch(function () {
                    depsDiv.innerHTML = '<div style="color:#CC2200;font-size:11px;">Failed to check dependencies</div>';
                });
        } else {
            depsDiv.innerHTML = "";
        }

        // Show env var inputs, pre-populated from existing config if available
        var envDiv = document.getElementById("install-modal-env");
        if (entry.config && entry.config.length > 0) {
            renderInstallEnvVars(envDiv, entry.config, {});
            // Try to load existing config to pre-populate values
            apiFetch("/api/mcp/config/" + encodeURIComponent(entry.id))
                .then(function (cfg) {
                    if (cfg && cfg.env) {
                        renderInstallEnvVars(envDiv, entry.config, cfg.env);
                    }
                })
                .catch(function () { /* no existing config, keep defaults */ });
        } else {
            envDiv.innerHTML = "";
        }

        installModal.style.display = "flex";
    }

    function closeInstallModal() {
        installModal.style.display = "none";
        currentInstallEntry = null;
    }

    document.getElementById("install-modal-close").addEventListener("click", closeInstallModal);
    document.getElementById("install-modal-cancel").addEventListener("click", closeInstallModal);

    document.getElementById("install-modal-confirm").addEventListener("click", function () {
        if (!currentInstallEntry) return;
        var entry = currentInstallEntry;
        var statusEl = document.getElementById("install-modal-status");
        statusEl.textContent = "Installing...";
        statusEl.className = "save-status";

        // Gather env values
        var envValues = {};
        document.querySelectorAll("#install-modal-env .install-env-input").forEach(function (input) {
            var name = input.getAttribute("data-env-name");
            var val = input.value.trim();
            if (val) envValues[name] = val;
        });

        apiFetch("/api/mcp/install", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ id: entry.id, env_values: envValues }),
        }).then(function () {
            statusEl.textContent = "Installed!";
            statusEl.className = "save-status save-ok";
            setTimeout(function () {
                closeInstallModal();
                loadUnifiedStore();
            }, 800);
        }).catch(function (err) {
            statusEl.textContent = "Error: " + err;
            statusEl.className = "save-status save-err";
        });
    });

    // Close modal on overlay click
    installModal.addEventListener("click", function (e) {
        if (e.target === installModal) closeInstallModal();
    });

    // =========================================================================
    // Configure Modal
    // =========================================================================
    var configureModal = document.getElementById("configure-modal");
    var currentConfigureName = null;

    function openConfigureModal(name) {
        currentConfigureName = name;
        document.getElementById("configure-modal-title").textContent = "Configure: " + name;
        document.getElementById("configure-modal-status").textContent = "";
        var fieldsDiv = document.getElementById("configure-modal-fields");
        fieldsDiv.innerHTML = '<div style="color:#888;font-size:12px;">Loading...</div>';
        configureModal.style.display = "flex";

        apiFetch("/api/mcp/config/" + encodeURIComponent(name))
            .then(function (cfg) {
                var html = '';
                html += '<div class="xp-field"><label>Command</label>' +
                    '<input type="text" class="xp-input xp-input-wide cfg-command" value="' + esc(cfg.command) + '"></div>';
                html += '<div class="xp-field"><label>Args (comma-sep)</label>' +
                    '<input type="text" class="xp-input xp-input-wide cfg-args" value="' + esc((cfg.args || []).join(", ")) + '"></div>';

                var envKeys = Object.keys(cfg.env || {});
                if (envKeys.length > 0) {
                    html += '<div style="font-size:11px;font-weight:bold;margin:8px 0 4px;">Environment variables:</div>';
                    envKeys.forEach(function (k) {
                        html += '<div class="xp-field">' +
                            '<label>' + escapeHtml(k) + '</label>' +
                            '<input type="text" class="xp-input xp-input-wide cfg-env-input" data-env-key="' + escapeAttr(k) + '" value="' + esc(cfg.env[k]) + '">' +
                            '</div>';
                    });
                }

                fieldsDiv.innerHTML = html;
            })
            .catch(function (err) {
                fieldsDiv.innerHTML = '<div style="color:#CC2200;font-size:12px;">Failed to load config: ' + escapeHtml(String(err)) + '</div>';
            });
    }

    function closeConfigureModal() {
        configureModal.style.display = "none";
        currentConfigureName = null;
    }

    document.getElementById("configure-modal-close").addEventListener("click", closeConfigureModal);
    document.getElementById("configure-modal-cancel").addEventListener("click", closeConfigureModal);

    document.getElementById("configure-modal-save").addEventListener("click", function () {
        if (!currentConfigureName) return;
        var statusEl = document.getElementById("configure-modal-status");
        statusEl.textContent = "Saving...";
        statusEl.className = "save-status";

        var command = configureModal.querySelector(".cfg-command").value.trim();
        var argsStr = configureModal.querySelector(".cfg-args").value.trim();
        var args = argsStr ? argsStr.split(",").map(function (s) { return s.trim(); }) : [];
        var env = {};
        configureModal.querySelectorAll(".cfg-env-input").forEach(function (input) {
            var key = input.getAttribute("data-env-key");
            env[key] = input.value.trim();
        });

        var body = { command: command, args: args, env: env, installed: true };

        apiFetch("/api/mcp/configure/" + encodeURIComponent(currentConfigureName), {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body),
        }).then(function () {
            statusEl.textContent = "Saved!";
            statusEl.className = "save-status save-ok";
            setTimeout(closeConfigureModal, 600);
        }).catch(function (err) {
            statusEl.textContent = "Error: " + err;
            statusEl.className = "save-status save-err";
        });
    });

    configureModal.addEventListener("click", function (e) {
        if (e.target === configureModal) closeConfigureModal();
    });

    // =========================================================================
    // Action Modal (reusable popup)
    // =========================================================================
    var actionModal = document.getElementById("action-modal");

    function openActionModal(title) {
        document.getElementById("action-modal-title").textContent = title;
        document.getElementById("action-modal-body").innerHTML =
            '<div class="action-modal-spinner">Working...</div>';
        actionModal.style.display = "flex";
    }

    function updateActionModal(html) {
        document.getElementById("action-modal-body").innerHTML = html;
    }

    function closeActionModal() {
        actionModal.style.display = "none";
    }

    document.getElementById("action-modal-close").addEventListener("click", closeActionModal);
    actionModal.addEventListener("click", function (e) {
        if (e.target === actionModal) closeActionModal();
    });

    function renderToolList(tools) {
        if (!tools || tools.length === 0) {
            return '<div style="padding:8px;color:#888;font-size:12px;">No tools reported</div>';
        }
        var html = '<div class="action-modal-tools">' +
            '<div class="action-modal-tools-header">' + tools.length + ' tools available</div>';
        for (var i = 0; i < tools.length; i++) {
            html += '<div class="action-modal-tool-row">' + escapeHtml(tools[i]) + '</div>';
        }
        return html + '</div>';
    }

    function renderStepHtml(steps) {
        var html = '';
        for (var i = 0; i < steps.length; i++) {
            var s = steps[i];
            var cls = 'action-modal-step action-modal-step-' + s.state;
            var icon = s.state === 'done' ? '&#10003;' : s.state === 'active' ? '&#9679;' : '&#10007;';
            html += '<div class="' + cls + '"><span class="action-modal-step-icon">' +
                icon + '</span> ' + escapeHtml(s.label) + '</div>';
        }
        return html;
    }

    // =========================================================================
    // Shared API helper
    // =========================================================================
    function apiFetch(url, opts) {
        opts = opts || {};
        var sep = url.indexOf("?") === -1 ? "?" : "&";
        var fullUrl = url + sep + "token=" + encodeURIComponent(TOKEN);
        var headers = opts.headers || {};
        headers["Authorization"] = "Bearer " + TOKEN;
        return fetch(fullUrl, Object.assign({}, opts, { headers: headers }))
            .then(function (resp) {
                if (!resp.ok) {
                    return resp.text().then(function (t) { throw new Error(t); });
                }
                return resp.json();
            });
    }

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

    function jsonPretty(obj) {
        if (!obj || (typeof obj === "object" && Object.keys(obj).length === 0)) return "{}";
        return JSON.stringify(obj, null, 2);
    }

    function tryParseJson(id) {
        var el = document.getElementById(id);
        var errEl = document.getElementById(id + "_err");
        if (!el) return {};
        var val = el.value.trim();
        if (!val || val === "{}") {
            if (errEl) errEl.textContent = "";
            return {};
        }
        try {
            var parsed = JSON.parse(val);
            if (errEl) errEl.textContent = "";
            return parsed;
        } catch (e) {
            if (errEl) errEl.textContent = "Invalid JSON: " + e.message;
            return null;
        }
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
