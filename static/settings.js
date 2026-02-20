(function() {
    "use strict";

    // --- Form population ---

    function loadForm(config) {
        setVal("general_data_dir", config.general.data_dir);
        setVal("general_log_level", config.general.log_level);

        setVal("llm_provider", config.llm.provider);
        setVal("llm_base_url", config.llm.base_url);
        setVal("llm_model", config.llm.model);
        setVal("llm_api_key_env", config.llm.api_key_env);
        setVal("llm_temperature", config.llm.temperature);
        setVal("llm_max_tokens", config.llm.max_tokens);
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
        var container = document.getElementById("mcpServers");
        container.innerHTML = "";
        if (config.mcp && config.mcp.servers) {
            Object.keys(config.mcp.servers).forEach(function(name) {
                var srv = config.mcp.servers[name];
                addMcpServerRow(name, srv.command, srv.args || [], srv.env || {});
            });
        }

        // Capabilities
        var capContainer = document.getElementById("capabilities");
        capContainer.innerHTML = "";
        (config.capabilities || []).forEach(function(cap) {
            addCapabilityRow(cap);
        });
    }

    function setVal(id, value) {
        var el = document.getElementById(id);
        if (el) el.value = value !== undefined && value !== null ? value : "";
    }

    function setChecked(id, value) {
        var el = document.getElementById(id);
        if (el) el.checked = !!value;
    }

    // --- Gather form back into config ---

    function gatherForm() {
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
                max_tokens: parseInt(getVal("llm_max_tokens"), 10) || 4096,
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
        document.querySelectorAll(".mcp-server-row").forEach(function(row) {
            var name = row.querySelector(".mcp-name").value.trim();
            if (!name) return;
            var command = row.querySelector(".mcp-command").value.trim();
            var argsStr = row.querySelector(".mcp-args").value.trim();
            var envStr = row.querySelector(".mcp-env").value.trim();
            var args = argsStr ? argsStr.split(",").map(function(s) { return s.trim(); }) : [];
            var env = {};
            if (envStr) {
                envStr.split(",").forEach(function(pair) {
                    var parts = pair.split("=");
                    if (parts.length === 2) env[parts[0].trim()] = parts[1].trim();
                });
            }
            config.mcp.servers[name] = { command: command, args: args, env: env };
        });

        // Gather capabilities
        document.querySelectorAll(".capability-row").forEach(function(row) {
            var cap = {
                name: row.querySelector(".cap-name").value.trim(),
                description: row.querySelector(".cap-description").value.trim(),
                schedule: row.querySelector(".cap-schedule").value.trim(),
                mcp_servers: row.querySelector(".cap-mcp-servers").value.trim().split(",").map(function(s) { return s.trim(); }).filter(Boolean),
                enabled: row.querySelector(".cap-enabled").checked,
            };
            var configPath = row.querySelector(".cap-config-path").value.trim();
            if (configPath) cap.config_path = configPath;
            if (cap.name) config.capabilities.push(cap);
        });

        return config;
    }

    function getVal(id) {
        var el = document.getElementById(id);
        return el ? el.value : "";
    }

    function getChecked(id) {
        var el = document.getElementById(id);
        return el ? el.checked : false;
    }

    // --- Dynamic MCP server rows ---

    function addMcpServerRow(name, command, args, env) {
        var container = document.getElementById("mcpServers");
        var row = document.createElement("div");
        row.className = "mcp-server-row dynamic-row";

        var envParts = [];
        if (env) {
            Object.keys(env).forEach(function(k) { envParts.push(k + "=" + env[k]); });
        }

        row.innerHTML =
            '<div class="dynamic-fields">' +
            '<div class="field"><label>Name</label><input type="text" class="mcp-name" value="' + esc(name || "") + '"></div>' +
            '<div class="field"><label>Command</label><input type="text" class="mcp-command" value="' + esc(command || "") + '"></div>' +
            '<div class="field"><label>Args (comma-separated)</label><input type="text" class="mcp-args" value="' + esc((args || []).join(", ")) + '"></div>' +
            '<div class="field"><label>Env (KEY=VAL, ...)</label><input type="text" class="mcp-env" value="' + esc(envParts.join(", ")) + '"></div>' +
            '</div>' +
            '<button type="button" class="btn btn-remove" onclick="this.parentNode.remove()">Remove</button>';
        container.appendChild(row);
    }

    function addCapabilityRow(cap) {
        var container = document.getElementById("capabilities");
        var row = document.createElement("div");
        row.className = "capability-row dynamic-row";
        cap = cap || {};
        row.innerHTML =
            '<div class="dynamic-fields">' +
            '<div class="field"><label>Name</label><input type="text" class="cap-name" value="' + esc(cap.name || "") + '"></div>' +
            '<div class="field"><label>Description</label><input type="text" class="cap-description" value="' + esc(cap.description || "") + '"></div>' +
            '<div class="field"><label>Schedule (cron)</label><input type="text" class="cap-schedule" value="' + esc(cap.schedule || "") + '"></div>' +
            '<div class="field"><label>MCP servers (comma-separated)</label><input type="text" class="cap-mcp-servers" value="' + esc((cap.mcp_servers || []).join(", ")) + '"></div>' +
            '<div class="field"><label>Config path</label><input type="text" class="cap-config-path" value="' + esc(cap.config_path || "") + '"></div>' +
            '<div class="field field-checkbox"><label><input type="checkbox" class="cap-enabled"' + (cap.enabled !== false ? " checked" : "") + '> Enabled</label></div>' +
            '</div>' +
            '<button type="button" class="btn btn-remove" onclick="this.parentNode.remove()">Remove</button>';
        container.appendChild(row);
    }

    function esc(str) {
        var div = document.createElement("div");
        div.textContent = str;
        return div.innerHTML.replace(/"/g, "&quot;");
    }

    // --- Save ---

    function saveSettings() {
        var config = gatherForm();
        var status = document.getElementById("saveStatus");
        status.textContent = "Saving...";
        status.className = "save-status";

        var xhr = new XMLHttpRequest();
        xhr.open("PUT", "/api/settings");
        xhr.setRequestHeader("Authorization", "Bearer " + EDITOR_TOKEN);
        xhr.setRequestHeader("Content-Type", "application/json");
        xhr.onload = function() {
            if (xhr.status === 200) {
                status.textContent = "Saved. Reloading...";
                status.className = "save-status save-ok";
                var newToken = config.news.editor_token || EDITOR_TOKEN;
                setTimeout(function() {
                    window.location.href = "/settings?token=" + encodeURIComponent(newToken);
                }, 500);
            } else {
                status.textContent = "Error: " + xhr.responseText;
                status.className = "save-status save-err";
            }
        };
        xhr.onerror = function() {
            status.textContent = "Network error.";
            status.className = "save-status save-err";
        };
        xhr.send(JSON.stringify(config));
    }

    // --- Init ---

    document.getElementById("settingsForm").addEventListener("submit", function(e) {
        e.preventDefault();
        saveSettings();
    });

    // Expose to inline onclick handlers
    window.addMcpServer = function() { addMcpServerRow("", "", [], {}); };
    window.addCapability = function() { addCapabilityRow({}); };

    loadForm(CONFIG);
})();
