(function() {
    "use strict";

    function esc(str) {
        var div = document.createElement("div");
        div.textContent = str;
        return div.innerHTML.replace(/"/g, "&quot;");
    }

    // --- Form building ---

    function loadForm(data) {
        var container = document.getElementById("sections");
        container.innerHTML = "";
        var briefing = data["daily-briefing"] || {};
        (briefing.sections || []).forEach(function(section) {
            addSectionRow(section);
        });
    }

    function addSectionRow(section) {
        section = section || {};
        var container = document.getElementById("sections");
        var fieldset = document.createElement("fieldset");
        fieldset.className = "topic-section";

        var title = section.title || "";
        fieldset.innerHTML =
            '<legend class="section-legend">' + esc(title || "New section") + '</legend>' +
            '<div class="section-body">' +
            '<div class="field"><label>Section title</label>' +
            '<input type="text" class="section-title" value="' + esc(title) + '"></div>' +
            '<div class="sources-container"></div>' +
            '<button type="button" class="btn btn-small add-source-btn">Add source</button>' +
            '</div>' +
            '<button type="button" class="btn btn-remove">Remove</button>';

        container.appendChild(fieldset);

        // Wire up legend update on title input
        var titleInput = fieldset.querySelector(".section-title");
        var legend = fieldset.querySelector(".section-legend");
        titleInput.addEventListener("input", function() {
            legend.textContent = titleInput.value || "New section";
        });

        // Wire up remove
        fieldset.querySelector(".btn-remove").addEventListener("click", function() {
            fieldset.remove();
        });

        // Wire up add source
        var sourcesContainer = fieldset.querySelector(".sources-container");
        fieldset.querySelector(".add-source-btn").addEventListener("click", function() {
            addSourceRow(sourcesContainer, {});
        });

        // Populate existing sources
        (section.sources || []).forEach(function(source) {
            addSourceRow(sourcesContainer, source);
        });
    }

    function addSourceRow(container, source) {
        source = source || {};
        var row = document.createElement("div");
        row.className = "source-row dynamic-row";

        var includeStr = (source.include || []).join(", ");
        var excludeStr = (source.exclude || []).join(", ");
        var freshness = source.freshness || "1d";
        var selectIf = source["select-if"] || "";

        row.innerHTML =
            '<div class="dynamic-fields">' +
            '<div class="field" style="grid-column:1/-1"><label>Search query</label>' +
            '<input type="text" class="src-search" value="' + esc(source.search || "") + '" style="max-width:100%"></div>' +
            '<div class="field"><label>Include terms (comma-separated)</label>' +
            '<input type="text" class="src-include" value="' + esc(includeStr) + '"></div>' +
            '<div class="field"><label>Exclude terms (comma-separated)</label>' +
            '<input type="text" class="src-exclude" value="' + esc(excludeStr) + '"></div>' +
            '<div class="field"><label>Freshness</label>' +
            '<select class="src-freshness">' +
            '<option value="1d"' + (freshness === "1d" ? " selected" : "") + '>Past day</option>' +
            '<option value="1w"' + (freshness === "1w" ? " selected" : "") + '>Past week</option>' +
            '<option value="1m"' + (freshness === "1m" ? " selected" : "") + '>Past month</option>' +
            '</select></div>' +
            '<div class="field" style="grid-column:1/-1"><label>Selection criteria</label>' +
            '<textarea class="src-select-if" rows="2" style="max-width:100%">' + esc(selectIf) + '</textarea></div>' +
            '</div>' +
            '<button type="button" class="btn btn-remove">Remove</button>';

        container.appendChild(row);

        row.querySelector(".btn-remove").addEventListener("click", function() {
            row.remove();
        });
    }

    // --- Gather form data ---

    function gatherForm() {
        var sections = [];
        document.querySelectorAll(".topic-section").forEach(function(fieldset) {
            var title = fieldset.querySelector(".section-title").value.trim();
            if (!title) return;

            var sources = [];
            fieldset.querySelectorAll(".source-row").forEach(function(row) {
                var search = row.querySelector(".src-search").value.trim();
                if (!search) return;

                var includeStr = row.querySelector(".src-include").value.trim();
                var excludeStr = row.querySelector(".src-exclude").value.trim();

                sources.push({
                    search: search,
                    include: includeStr ? includeStr.split(",").map(function(s) { return s.trim(); }).filter(Boolean) : [],
                    exclude: excludeStr ? excludeStr.split(",").map(function(s) { return s.trim(); }).filter(Boolean) : [],
                    "select-if": row.querySelector(".src-select-if").value.trim(),
                    freshness: row.querySelector(".src-freshness").value,
                });
            });

            sections.push({ title: title, sources: sources });
        });

        return { "daily-briefing": { sections: sections } };
    }

    // --- Save ---

    function saveTopics() {
        var data = gatherForm();
        var yaml = jsyaml.dump(data, { lineWidth: -1 });

        var status = document.getElementById("saveStatus");
        status.textContent = "Saving...";
        status.className = "save-status";

        var xhr = new XMLHttpRequest();
        xhr.open("PUT", "/api/topics");
        xhr.setRequestHeader("Authorization", "Bearer " + EDITOR_TOKEN);
        xhr.setRequestHeader("Content-Type", "text/plain");
        xhr.onload = function() {
            if (xhr.status === 200) {
                status.textContent = "Saved. Reloading...";
                status.className = "save-status save-ok";
                setTimeout(function() {
                    window.location.href = "/settings/topics?token=" + encodeURIComponent(EDITOR_TOKEN);
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
        xhr.send(yaml);
    }

    // --- Init ---

    document.getElementById("saveTopics").addEventListener("click", saveTopics);
    document.getElementById("addSection").addEventListener("click", function() {
        addSectionRow({});
    });

    loadForm(TOPICS);
})();
