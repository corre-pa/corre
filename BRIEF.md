# Overall goal

* An AI environment that lets users schedule complex tasks/capabilities and have the output served up in a "newspaper" type format
  (CorreNews).
* CorreNews is served locally, but accessible remotely via a secure NAT punching method. So users can access their CorreNews from anywhere,
  but the data is stored locally and not shared with 3rd parties.
* Corre management is done via a TUI (via ssh or local terminal)
* Writing new capabilities is done using a separate, but integrated tool.
* Archive daily news editions so that the user can look back on them.
* Index content for search and retrieval later.

## Philosophy

* Rust is the primary language where reasonable.
* Be cognizant of personal data being shared with 3rd party LLMs. Use privacy-focused tools where possible. E.g. Venice for LLMs, Kagi
  for search, etc.
* Flexibility and modularity. Not tied to a single platform / LLM. Allow multiple APIs, models etc. Let people do what they want to do.
* Security. Only perform actions explicitly authorized by the user. Look out for and prevent prompt injection and other attacks.
* Complex capabilities are similar to Skills in Claude Code. A natural-language description offloads tasks to MCP servers, and
  then the output is sent to a publishing pipeline that converts the output into beautifully rendered newspaper style HTML for
  CorreNews.
* The heartbeat and scheduling should NOT be done through LLM calls. CronJobs or similar scheduling tools should be used.
* First-class support for helping to write build and deploy MCP servers is a core part of the project.
* Support for existing MCP servers.
* Best-in-class episodic and semantic memory.
* Use existing tools that meet the functional requirements and match with our philosophy. Don't reinvent the wheel.
    * IronClaw, for example offers many features that we want, but has vendor lockin (https://github.com/nearai/ironclaw)
    * The oOfficial Rust MCP SDK https://github.com/modelcontextprotocol/rust-sdk
    * Windmill might also offer things we can re-use: https://github.com/windmill-labs/windmill

## Examples

Here are some of the tasks Corre might be able to do. Each capability should be completely modular and installable / removable.
Each task might have several MCP dependencies. Only install MCP servers once. If removing a capability, only remove an MCP server if no
other capability depends on it.

## Daily research brief:

* Each morning at 5:00, search the web for latest new and developments on topics in "topics.md";
* Evaluate what's new / newsworthy according to criteria in topics.md
* and then compiles a customized newspaper that It serves up on a local server (call it CorreNews)
* (which I access via a secure NAT punching method).

## Stock portfolio review

* Each day after market close, pull stock data for stocks in "portfolio.md"
* Evaluate performance according to criteria in "portfolio.md"
* Look for any news or announcements that may affect the price of the stocks in "portfolio.md"
* Hit e.g., SimpleWallStreet's API to get sentiment analysis on the stocks in "portfolio.md"
* Compile a report that is served up on CorreNews

## Fantasy Sports Assistant

* Once a week, pull data on my fantasy sports teams
* Evaluate performance, look for news on players, etc. according to criteria in "fantasy.md"
* Run the data through my fantasy model (a lua or python script that the user can provide) to get recommendations on how to set my lineup,
  etc.
* Compile a report that is served up on CorreNews
    * The report includes a CTA button that lets me trigger a task that runs a script to set my lineup according to the model's
      recommendations.

## Birthday Reminder

* Pull birthday data from "birthdays.md"
* Each morning, check if there are any birthdays that day
* Compose a short message wishing the person a happy birthday.
* Send it to them on WhatsApp (or Signal, or Telegram, etc. depending on their preference, which is stored in "birthdays.md")
* Write a summary of what it sent on CorreNews.

## Import existing MCP servers

Adding a capability from an existing MCP server should be as easy as appending the MCP details to the corre.toml config file.