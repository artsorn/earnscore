export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // Helper to sanitize chat/message properties recursively
    const sanitizeObj = (val) => {
      if (val === null || val === undefined) return val;
      if (Array.isArray(val)) {
        return val.map(sanitizeObj);
      }
      if (typeof val === "object") {
        const cleaned = {};
        for (const key of Object.keys(val)) {
          const lower = key.toLowerCase();
          const isChat = lower === "chat" ||
                         lower === "message" ||
                         lower === "messages" ||
                         lower === "comment" ||
                         lower === "comments" ||
                         lower === "messageroom" ||
                         lower === "commentroom" ||
                         lower === "chatroom" ||
                         lower.includes("chat") ||
                         lower.includes("message") ||
                         lower.includes("comment");
          if (!isChat) {
            cleaned[key] = sanitizeObj(val[key]);
          }
        }
        return cleaned;
      }
      return val;
    };

    // Authentication helper
    const checkAuth = async (req) => {
      const authHeader = req.headers.get("Authorization");
      if (!authHeader || !authHeader.startsWith("Bearer ")) {
        return false;
      }
      const token = authHeader.substring(7);
      
      // Get token from DB
      try {
        const { value } = await env.DB.prepare("SELECT value FROM settings WHERE key = 'api_token'").first() || {};
        return token === (value || env.API_TOKEN);
      } catch (e) {
        return token === env.API_TOKEN;
      }
    };

    // 1. API: Receive synced data from Rust CLI
    if (url.pathname === "/api/sync" && request.method === "POST") {
      if (!(await checkAuth(request))) {
        return new Response(JSON.stringify({ error: "Unauthorized" }), { status: 401, headers: { "Content-Type": "application/json" } });
      }

      try {
        const rawBody = await request.text();
        let data;
        try {
          data = JSON.parse(rawBody);
        } catch (e) {
          return new Response(JSON.stringify({ error: "Invalid JSON payload" }), { status: 400, headers: { "Content-Type": "application/json" } });
        }

        const { matches, match_details, teams, competitions } = data;
        const statements = [];
        const syncedIds = {
          competitions: [],
          teams: [],
          matches: [],
          match_details: []
        };

        // Save competitions
        if (competitions && Array.isArray(competitions)) {
          for (const c of competitions) {
            if (!c.id || !c.sport_id || !c.name) continue;
            const cleaned = sanitizeObj(c);
            statements.push(
              env.DB.prepare(
                "INSERT INTO competitions (id, sport_id, name, logo, slug, country_name, country_logo, raw_payload, synced, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, datetime('now')) ON CONFLICT(id) DO UPDATE SET name=excluded.name, logo=excluded.logo, slug=excluded.slug, country_name=excluded.country_name, country_logo=excluded.country_logo, raw_payload=excluded.raw_payload, synced=1, updated_at=datetime('now')"
              ).bind(cleaned.id, cleaned.sport_id, cleaned.name, cleaned.logo || null, cleaned.slug || null, cleaned.country_name || null, cleaned.country_logo || null, JSON.stringify(cleaned))
            );
            syncedIds.competitions.push(c.id);
          }
        }

        // Save teams
        if (teams && Array.isArray(teams)) {
          for (const t of teams) {
            if (!t.id || !t.sport_id || !t.name) continue;
            const cleaned = sanitizeObj(t);
            statements.push(
              env.DB.prepare(
                "INSERT INTO teams (id, sport_id, name, logo, slug, raw_payload, synced, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, datetime('now')) ON CONFLICT(id) DO UPDATE SET name=excluded.name, logo=excluded.logo, slug=excluded.slug, raw_payload=excluded.raw_payload, synced=1, updated_at=datetime('now')"
              ).bind(cleaned.id, cleaned.sport_id, cleaned.name, cleaned.logo || null, cleaned.slug || null, JSON.stringify(cleaned))
            );
            syncedIds.teams.push(t.id);
          }
        }

        // Save matches
        if (matches && Array.isArray(matches)) {
          for (const m of matches) {
            if (!m.id || !m.sport_id || !m.competition_id || !m.home_team_id || !m.away_team_id) continue;
            const cleaned = sanitizeObj(m);
            statements.push(
              env.DB.prepare(
                "INSERT INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, raw_payload, synced, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, datetime('now')) ON CONFLICT(id) DO UPDATE SET sport_id=excluded.sport_id, competition_id=excluded.competition_id, home_team_id=excluded.home_team_id, away_team_id=excluded.away_team_id, match_time=excluded.match_time, status_id=excluded.status_id, home_scores=excluded.home_scores, away_scores=excluded.away_scores, raw_payload=excluded.raw_payload, synced=1, updated_at=datetime('now')"
              ).bind(cleaned.id, cleaned.sport_id, cleaned.competition_id, cleaned.home_team_id, cleaned.away_team_id, cleaned.match_time, cleaned.status_id, cleaned.home_scores, cleaned.away_scores, JSON.stringify(cleaned))
            );
            syncedIds.matches.push(m.id);
          }
        }

        // Save match details
        if (match_details && Array.isArray(match_details)) {
          for (const d of match_details) {
            if (!d.match_id || !d.sport_id) continue;
            const cleaned = sanitizeObj(d);
            statements.push(
              env.DB.prepare(
                "INSERT INTO match_details (match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, synced, last_updated, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, datetime('now')) ON CONFLICT(match_id) DO UPDATE SET incidents=excluded.incidents, stats=excluded.stats, lineups=excluded.lineups, odds=excluded.odds, h2h=excluded.h2h, raw_payload=excluded.raw_payload, synced=1, last_updated=excluded.last_updated, updated_at=datetime('now')"
              ).bind(cleaned.match_id, cleaned.sport_id, cleaned.incidents, cleaned.stats, cleaned.lineups, cleaned.odds, cleaned.h2h, JSON.stringify(cleaned), cleaned.last_updated || null)
            );
            syncedIds.match_details.push(d.match_id);
          }
        }

        if (statements.length > 0) {
          await env.DB.batch(statements);
        }

        // Read active sync_interval_mins
        let syncIntervalMins = 5;
        try {
          const settingRow = await env.DB.prepare("SELECT value FROM settings WHERE key = 'sync_interval_mins'").first();
          if (settingRow && settingRow.value) {
            const parsed = parseInt(settingRow.value, 10);
            if (!isNaN(parsed)) syncIntervalMins = Math.min(Math.max(parsed, 1), 60);
          }
        } catch (_) {}

        return new Response(JSON.stringify({
          success: true,
          sync_interval_mins: syncIntervalMins,
          synced_ids: syncedIds
        }), {
          headers: { "Content-Type": "application/json" }
        });
      } catch (e) {
        return new Response(JSON.stringify({ error: e.message }), {
          status: 400,
          headers: { "Content-Type": "application/json" }
        });
      }
    }

    // 2. API: Fetch live/today matches
    if (url.pathname === "/api/matches/live" && request.method === "GET") {
      try {
        const sportFilter = url.searchParams.get("sport_id");
        const parsedSportFilter = sportFilter === null ? null : Number(sportFilter);
        if (parsedSportFilter !== null && ![1, 2].includes(parsedSportFilter)) {
          return new Response(JSON.stringify({ error: "sport_id must be 1 or 2" }), {
            status: 400,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
          });
        }
        let query = `
          SELECT 
            m.id, m.sport_id, m.home_team_id, m.away_team_id, m.match_time, m.status_id, m.home_scores, m.away_scores, m.updated_at, m.raw_payload,
            md.updated_at as detail_updated_at,
            ht.name as home_name, ht.logo as home_logo, ht.slug as home_slug,
            at.name as away_name, at.logo as away_logo, at.slug as away_slug,
            c.name as comp_name, c.logo as comp_logo, c.country_name, c.country_logo
          FROM matches m
          LEFT JOIN teams ht ON m.home_team_id = ht.id
          LEFT JOIN teams at ON m.away_team_id = at.id
          LEFT JOIN competitions c ON m.competition_id = c.id
          LEFT JOIN match_details md ON m.id = md.match_id
        `;
        const predicates = [
          `(
            (m.sport_id = 1 AND m.status_id IN (2, 3, 4, 5, 6))
            OR (m.sport_id = 2 AND m.status_id IN (2, 3, 4, 5, 6, 7, 8, 9))
            OR (
              m.match_time >= unixepoch('now', 'start of day')
              AND m.match_time < unixepoch('now', 'start of day', '+1 day')
            )
          )`
        ];
        const params = [];
        if (parsedSportFilter !== null) {
          predicates.push("m.sport_id = ?1");
          params.push(parsedSportFilter);
        }
        query += " WHERE " + predicates.join(" AND ") + " ORDER BY m.match_time ASC, m.id ASC";

        const stmt = env.DB.prepare(query);
        const { results } = await (params.length > 0 ? stmt.bind(...params).all() : stmt.all());

        // Format and parse arrays safely
        const formatted = results.map(row => {
          let homeScores = [];
          try {
            homeScores = JSON.parse(row.home_scores || "[]");
          } catch (_) {}
          let awayScores = [];
          try {
            awayScores = JSON.parse(row.away_scores || "[]");
          } catch (_) {}
          let rawPayload = {};
          try {
            rawPayload = JSON.parse(row.raw_payload || "{}");
          } catch (_) {}

          return {
            ...row,
            home_scores: homeScores,
            away_scores: awayScores,
            raw_payload: rawPayload
          };
        });

        return new Response(JSON.stringify(formatted), {
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
        });
      } catch (e) {
        return new Response(JSON.stringify({ error: e.message }), {
          status: 500,
          headers: { "Content-Type": "application/json" }
        });
      }
    }

    // 2b. API: Fetch match details (stats, lineups, incidents)
    if (url.pathname === "/api/matches/detail" && request.method === "GET") {
      try {
        const matchId = url.searchParams.get("match_id");
        if (!matchId) {
          return new Response(JSON.stringify({ error: "Missing match_id" }), { status: 400, headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" } });
        }
        const result = await env.DB.prepare(
          "SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, last_updated FROM match_details WHERE match_id = ?1"
        ).bind(matchId).first();

        if (!result) {
          return new Response(JSON.stringify({ error: "Match details not found" }), {
            status: 404,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
          });
        }

        const safeParse = (str, fallback) => {
          try {
            return JSON.parse(str || fallback);
          } catch (_) {
            return JSON.parse(fallback);
          }
        };

        const rawPayload = sanitizeObj(safeParse(result.raw_payload, "{}"));
        const knownDetailKeys = new Set([
          "match_id", "matchId", "sport_id", "sportId", "incidents", "stats",
          "lineups", "odds", "h2h", "raw_payload", "last_updated", "lastUpdated",
          "synced", "updated_at"
        ]);
        const extra = rawPayload && typeof rawPayload === "object" && !Array.isArray(rawPayload)
          ? Object.fromEntries(Object.entries(rawPayload).filter(([key]) => !knownDetailKeys.has(key)))
          : {};
        const formatted = {
          ...result,
          incidents: safeParse(result.incidents, "[]"),
          stats: safeParse(result.stats, "{}"),
          lineups: safeParse(result.lineups, "{}"),
          odds: safeParse(result.odds, "{}"),
          h2h: safeParse(result.h2h, "{}"),
          raw_payload: rawPayload,
          extra
        };

        return new Response(JSON.stringify(formatted), {
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
        });
      } catch (e) {
        return new Response(JSON.stringify({ error: e.message }), {
          status: 500,
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
        });
      }
    }

    // 3. API: Get & Post Settings
    if (url.pathname === "/api/settings") {
      if (request.method === "GET") {
        try {
          const rows = await env.DB.prepare("SELECT key, value FROM settings").all();
          const settings = {};
          rows.results.forEach(r => {
            if (r.key !== "api_token" && r.key !== "uploader_lease_owner" && r.key !== "uploader_lease_expires") { // Hide secret tokens/lease keys
              settings[r.key] = r.value;
            }
          });
          return new Response(JSON.stringify(settings), {
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
          });
        } catch (e) {
          return new Response(JSON.stringify({ error: e.message }), { status: 500, headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" } });
        }
      }

      if (request.method === "POST") {
        if (!(await checkAuth(request))) {
          return new Response(JSON.stringify({ error: "Unauthorized" }), { status: 401, headers: { "Content-Type": "application/json" } });
        }

        try {
          const body = await request.json();
          const { sync_interval_mins, detail_update_interval_secs, api_token } = body;

          const statements = [];
          if (sync_interval_mins !== undefined) {
            const val = Number(sync_interval_mins);
            if (!Number.isInteger(val) || val < 1 || val > 60) {
              return new Response(JSON.stringify({ error: "sync_interval_mins must be an integer from 1 to 60" }), { status: 400, headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" } });
            }
            statements.push(
              env.DB.prepare("INSERT OR REPLACE INTO settings (key, value) VALUES ('sync_interval_mins', ?1)").bind(String(val))
            );
          }
          if (detail_update_interval_secs !== undefined) {
            const val = Number(detail_update_interval_secs);
            if (!Number.isInteger(val) || (val !== 0 && (val < 5 || val > 3600))) {
              return new Response(JSON.stringify({ error: "detail_update_interval_secs must be 0 or an integer from 5 to 3600" }), { status: 400, headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" } });
            }
            statements.push(
              env.DB.prepare("INSERT OR REPLACE INTO settings (key, value) VALUES ('detail_update_interval_secs', ?1)").bind(String(val))
            );
          }
          if (api_token && api_token.trim() !== "") {
            statements.push(
              env.DB.prepare("INSERT OR REPLACE INTO settings (key, value) VALUES ('api_token', ?1)").bind(api_token.trim())
            );
          }

          if (statements.length > 0) {
            await env.DB.batch(statements);
          }

          return new Response(JSON.stringify({ success: true }), {
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
          });
        } catch (e) {
          return new Response(JSON.stringify({ error: e.message }), { status: 400, headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" } });
        }
      }
    }

    // 4. GUI: Serve Web Interface Dashboard
    if (url.pathname === "/" && request.method === "GET") {
      const html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>EarnScore - Live Matches Dashboard</title>
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Outfit:wght@300;400;600;800&display=swap" rel="stylesheet">
  <style>
    :root {
      --bg: #0b0816;
      --panel: #141026;
      --card: #1f1b3a;
      --text: #f1ecf9;
      --text-dim: #9a92ab;
      --primary: #8a2be2;
      --primary-light: #a76df2;
      --accent: #ff007f;
      --success: #00ffcc;
      --football: #00f0ff;
      --basketball: #ffa800;
    }

    * {
      box-sizing: border-box;
      margin: 0;
      padding: 0;
    }

    body {
      font-family: 'Outfit', sans-serif;
      background-color: var(--bg);
      color: var(--text);
      min-height: 100vh;
      display: flex;
      flex-direction: column;
      overflow-x: hidden;
    }

    header {
      background: linear-gradient(135deg, #180d35 0%, var(--bg) 100%);
      padding: 1.5rem 2rem;
      display: flex;
      justify-content: space-between;
      align-items: center;
      border-bottom: 1px solid #231b47;
      box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
    }

    .logo-container {
      display: flex;
      align-items: center;
      gap: 10px;
    }

    .logo-text {
      font-size: 1.8rem;
      font-weight: 800;
      letter-spacing: 1px;
      background: linear-gradient(95deg, var(--football) 0%, var(--primary-light) 50%, var(--accent) 100%);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
    }

    main {
      flex: 1;
      padding: 2rem;
      max-width: 1400px;
      width: 100%;
      margin: 0 auto;
    }

    .sports-sections {
      display: grid;
      grid-template-columns: 1fr;
      gap: 2rem;
    }

    @media (min-width: 1024px) {
      .sports-sections {
        grid-template-columns: 1fr 1fr;
      }
    }

    .section-card {
      background-color: var(--panel);
      border-radius: 16px;
      padding: 1.5rem;
      border: 1px solid #231d45;
      box-shadow: 0 15px 40px rgba(0, 0, 0, 0.3);
      display: flex;
      flex-direction: column;
      min-height: 400px;
    }

    .section-title {
      font-size: 1.4rem;
      font-weight: 800;
      margin-bottom: 1.5rem;
      display: flex;
      justify-content: space-between;
      align-items: center;
      border-bottom: 2px solid #231b47;
      padding-bottom: 0.8rem;
    }

    .football-title {
      color: var(--football);
    }

    .basketball-title {
      color: var(--basketball);
    }

    .match-list {
      display: flex;
      flex-direction: column;
      gap: 15px;
      overflow-y: auto;
      max-height: 70vh;
      padding-right: 5px;
    }

    .match-card {
      background-color: var(--card);
      border-radius: 12px;
      padding: 1rem;
      border: 1px solid #2d265a;
      transition: all 0.3s ease;
      display: flex;
      flex-direction: column;
      gap: 10px;
      cursor: pointer;
    }

    .match-card:hover, .match-card:focus-visible {
      border-color: var(--primary-light);
      transform: scale(1.01);
      box-shadow: 0 5px 15px rgba(0, 0, 0, 0.4);
      outline: none;
    }

    .match-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      font-size: 0.85rem;
      color: var(--text-dim);
      border-bottom: 1px solid #2f275e;
      padding-bottom: 6px;
    }

    .league-info {
      display: flex;
      align-items: center;
      gap: 6px;
    }

    .league-logo {
      width: 16px;
      height: 16px;
      border-radius: 50%;
      object-fit: cover;
    }

    .match-status {
      padding: 2px 8px;
      border-radius: 12px;
      font-size: 0.75rem;
      font-weight: 800;
      text-transform: uppercase;
    }

    .status-live {
      background-color: rgba(255, 0, 127, 0.15);
      color: var(--accent);
      border: 1px solid rgba(255, 0, 127, 0.3);
      animation: pulse 1.5s infinite;
    }

    .status-ft {
      background-color: rgba(0, 255, 204, 0.1);
      color: var(--success);
      border: 1px solid rgba(0, 255, 204, 0.2);
    }

    .status-upcoming {
      background-color: rgba(154, 146, 171, 0.1);
      color: var(--text-dim);
    }

    .status-unknown {
      background-color: rgba(255, 168, 0, 0.12);
      color: var(--basketball);
      border: 1px solid rgba(255, 168, 0, 0.25);
    }

    #sync-info.refresh-error {
      color: #ff669f !important;
    }

    #sync-info.refresh-stale {
      color: var(--basketball) !important;
    }

    .match-body {
      display: grid;
      grid-template-columns: 1fr auto 1fr;
      align-items: center;
      gap: 10px;
    }

    .team {
      display: flex;
      align-items: center;
      gap: 12px;
    }

    .team-home {
      justify-content: flex-end;
      text-align: right;
    }

    .team-away {
      justify-content: flex-start;
      text-align: left;
    }

    .team-logo {
      width: 32px;
      height: 32px;
      object-fit: contain;
    }

    .team-name {
      font-weight: 600;
      font-size: 0.95rem;
    }

    .score-area {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 4px;
      min-width: 80px;
    }

    .score-live {
      font-size: 1.5rem;
      font-weight: 800;
      letter-spacing: 2px;
      background: linear-gradient(180deg, #fff 0%, #d5cde6 100%);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
    }

    .score-half {
      font-size: 0.75rem;
      color: var(--text-dim);
    }

    .match-time {
      font-size: 0.85rem;
      color: var(--text-dim);
    }

    .empty-state {
      text-align: center;
      color: var(--text-dim);
      padding: 3rem 1rem;
      font-size: 1rem;
      margin: auto;
    }

    .settings-btn {
      background: transparent;
      border: none;
      color: var(--text-dim);
      cursor: pointer;
      font-size: 1.5rem;
      transition: color 0.3s;
    }

    .settings-btn:hover {
      color: var(--text);
    }

    .modal {
      display: none;
      position: fixed;
      top: 0;
      left: 0;
      width: 100%;
      height: 100%;
      background: rgba(0,0,0,0.7);
      backdrop-filter: blur(5px);
      justify-content: center;
      align-items: center;
      z-index: 100;
    }

    .modal-content {
      background: var(--panel);
      border: 1px solid #342a63;
      border-radius: 16px;
      padding: 2rem;
      width: 90%;
      max-width: 450px;
      box-shadow: 0 20px 50px rgba(0,0,0,0.5);
      position: relative;
    }

    .modal-title {
      font-size: 1.4rem;
      font-weight: 800;
      margin-bottom: 1.5rem;
    }

    .form-group {
      margin-bottom: 1.2rem;
      display: flex;
      flex-direction: column;
      gap: 6px;
    }

    .form-group label {
      font-size: 0.9rem;
      color: var(--text-dim);
      font-weight: 600;
    }

    .form-group input, .form-group select {
      background: var(--bg);
      border: 1px solid #322b5e;
      color: white;
      padding: 10px;
      border-radius: 8px;
      font-family: inherit;
    }

    .form-group input:focus, .form-group select:focus {
      border-color: var(--primary-light);
      outline: none;
    }

    .form-help {
      font-size: 0.75rem;
      color: var(--text-dim);
      line-height: 1.3;
    }

    .modal-actions {
      display: flex;
      justify-content: flex-end;
      gap: 10px;
      margin-top: 2rem;
    }

    .btn {
      padding: 10px 20px;
      border-radius: 8px;
      cursor: pointer;
      font-family: inherit;
      font-weight: 600;
      border: none;
    }

    .btn-cancel {
      background: #322b5e;
      color: var(--text-dim);
    }

    .btn-save {
      background: linear-gradient(135deg, var(--primary) 0%, var(--primary-light) 100%);
      color: white;
    }

    .alert-banner {
      padding: 10px;
      border-radius: 8px;
      margin-bottom: 15px;
      font-size: 0.85rem;
      display: none;
    }
    .alert-success {
      background-color: rgba(0, 255, 204, 0.15);
      border: 1px solid var(--success);
      color: var(--success);
    }
    .alert-error {
      background-color: rgba(255, 0, 127, 0.15);
      border: 1px solid var(--accent);
      color: #ff3399;
    }

    @keyframes pulse {
      0% { opacity: 0.8; transform: scale(1); }
      50% { opacity: 1; transform: scale(1.02); }
      100% { opacity: 0.8; transform: scale(1); }
    }

    @media (prefers-reduced-motion: reduce) {
      .status-live {
        animation: none;
      }
      .match-card:hover {
        transform: none;
      }
    }

    ::-webkit-scrollbar {
      width: 6px;
    }
    ::-webkit-scrollbar-track {
      background: var(--bg);
    }
    ::-webkit-scrollbar-thumb {
      background: var(--card);
      border-radius: 3px;
    }

    .red-card-badge {
      background-color: #ff0055;
      color: white;
      font-size: 0.7rem;
      padding: 1px 4px;
      border-radius: 3px;
      font-weight: bold;
      margin-left: 5px;
      display: inline-block;
      vertical-align: middle;
    }
    .yellow-card-badge {
      background-color: #ffcc00;
      color: black;
      font-size: 0.7rem;
      padding: 1px 4px;
      border-radius: 3px;
      font-weight: bold;
      margin-left: 5px;
      display: inline-block;
      vertical-align: middle;
    }
    .corners-info {
      font-size: 0.75rem;
      color: var(--text-dim);
      display: flex;
      align-items: center;
      gap: 4px;
      margin-top: 2px;
      justify-content: center;
    }
    .basketball-quarters {
      font-size: 0.75rem;
      color: var(--football);
      margin-top: 2px;
      letter-spacing: 0.5px;
      text-align: center;
    }
    
    .match-details-container {
      margin-top: 10px;
      padding-top: 10px;
      border-top: 1px dashed #2f275e;
      display: none;
      animation: fadeIn 0.3s ease;
    }
    .match-card.expanded .match-details-container {
      display: block;
    }
    
    @keyframes fadeIn {
      from { opacity: 0; transform: translateY(-10px); }
      to { opacity: 1; transform: translateY(0); }
    }
    
    .detail-section-title {
      font-size: 0.8rem;
      font-weight: 800;
      color: var(--football);
      margin-top: 10px;
      margin-bottom: 6px;
      text-transform: uppercase;
      letter-spacing: 1px;
    }
    
    .stats-grid {
      display: flex;
      flex-direction: column;
      gap: 6px;
      margin-bottom: 10px;
    }
    .stat-row {
      display: grid;
      grid-template-columns: 2fr 3fr 2fr;
      align-items: center;
      text-align: center;
      font-size: 0.8rem;
    }
    .stat-bar-container {
      grid-column: 1 / span 3;
      height: 4px;
      background: #191433;
      border-radius: 2px;
      display: flex;
      overflow: hidden;
      margin-top: 3px;
      margin-bottom: 3px;
    }
    .stat-bar-home {
      height: 100%;
      background: var(--football);
    }
    .stat-bar-away {
      height: 100%;
      background: var(--accent);
    }
    
    .timeline-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
      margin-bottom: 10px;
      position: relative;
      padding-left: 10px;
      border-left: 1px solid #2f275e;
    }
    .timeline-item {
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 0.75rem;
    }
    .timeline-time {
      font-weight: bold;
      color: var(--success);
      min-width: 25px;
    }
    .timeline-icon {
      font-size: 0.9rem;
    }
    
    .lineups-container {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 10px;
      font-size: 0.75rem;
    }
    .lineup-team-title {
      font-weight: bold;
      margin-bottom: 4px;
      color: var(--text);
      border-bottom: 1px solid #231d45;
      padding-bottom: 2px;
    }
    .player-row {
      display: flex;
      justify-content: space-between;
      padding: 2px 0;
      color: var(--text-dim);
    }

    .detail-list {
      display: flex;
      flex-direction: column;
      gap: 6px;
      padding-left: 1.2rem;
      margin-bottom: 10px;
    }

    .detail-grid {
      display: grid;
      grid-template-columns: minmax(90px, 1fr) minmax(0, 2fr);
      gap: 5px 10px;
      margin-bottom: 10px;
      font-size: 0.78rem;
    }

    .detail-grid dt {
      color: var(--text-dim);
      font-weight: 600;
      overflow-wrap: anywhere;
    }

    .detail-grid dd {
      min-width: 0;
      color: var(--text);
      overflow-wrap: anywhere;
    }
    
    .loading-spinner {
      text-align: center;
      padding: 15px;
      color: var(--text-dim);
      font-size: 0.8rem;
    }
  </style>
</head>
<body>

  <header>
    <div class="logo-container">
      <span class="logo-text">EarnScore</span>
      <span style="font-size: 0.8rem; background: #2f1d52; padding: 2px 8px; border-radius: 10px; color: var(--success); font-weight: 800;">LIVE D1</span>
    </div>
    
    <div style="display: flex; align-items: center; gap: 20px;">
      <div id="sync-info" role="status" aria-live="polite" style="font-size: 0.85rem; color: var(--text-dim); text-align: right;">
        Last refresh: --:--
      </div>
      <button class="settings-btn" id="open-settings" aria-label="System Settings">⚙️</button>
    </div>
  </header>

  <main>
    <div class="sports-sections">
      <section class="section-card" aria-labelledby="football-section-title">
        <h2 class="section-title football-title" id="football-section-title">
          <span>⚽ Football Live / Today</span>
          <span id="football-count" style="font-size: 1rem; color: var(--text-dim);">0 matches</span>
        </h2>
        <div class="match-list" id="football-matches">
          <div class="empty-state">Loading matches...</div>
        </div>
      </section>

      <section class="section-card" aria-labelledby="basketball-section-title">
        <h2 class="section-title basketball-title" id="basketball-section-title">
          <span>🏀 Basketball Live / Today</span>
          <span id="basketball-count" style="font-size: 1rem; color: var(--text-dim);">0 matches</span>
        </h2>
        <div class="match-list" id="basketball-matches">
          <div class="empty-state">Loading matches...</div>
        </div>
      </section>
    </div>
  </main>

  <div class="modal" id="settings-modal" role="dialog" aria-modal="true" aria-labelledby="modal-settings-title">
    <div class="modal-content">
      <h3 class="modal-title" id="modal-settings-title">⚙️ System Settings</h3>
      <div id="modal-alert" class="alert-banner" role="status" aria-live="polite"></div>

      <div class="form-group">
        <label for="sync-interval">D1 Sync Interval (every X minutes)</label>
        <input type="number" id="sync-interval" min="1" max="60" value="5">
        <span class="form-help">Controls SQLite to D1 background sync interval on the server.</span>
      </div>
      <div class="form-group">
        <label for="detail-interval">Match Details Update Interval</label>
        <select id="detail-interval">
          <option value="5">5 Seconds</option>
          <option value="15">15 Seconds</option>
          <option value="30">30 Seconds</option>
          <option value="60">1 Minute (60s)</option>
          <option value="120">2 Minutes</option>
          <option value="240">4 Minutes</option>
          <option value="480">8 Minutes</option>
          <option value="960">16 Minutes</option>
          <option value="0">First & Last Time Only (No Live Updates)</option>
        </select>
        <span class="form-help">Controls crawler detail fetches. The browser dashboard refreshes separately every 10 seconds while visible.</span>
      </div>
      <div class="form-group">
        <label for="api-token">Authorization API Token</label>
        <input type="password" id="api-token" placeholder="••••••••">
        <span class="form-help">Used only to authorize this save request. It is not stored in the browser or returned by the API.</span>
      </div>
      <div class="modal-actions">
        <button class="btn btn-cancel" id="close-settings">Cancel</button>
        <button class="btn btn-save" id="save-settings">Save Settings</button>
      </div>
    </div>
  </div>

  <script>
    const BASE_POLL_MS = 10000;
    const MAX_POLL_MS = 120000;
    const DEFAULT_LOGOS = {
      1: 'data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2232%22 height=%2232%22 viewBox=%220 0 24 24%22%3E%3Ccircle cx=%2212%22 cy=%2212%22 r=%2210%22 fill=%22%2300f0ff%22/%3E%3C/svg%3E',
      2: 'data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2232%22 height=%2232%22 viewBox=%220 0 24 24%22%3E%3Ccircle cx=%2212%22 cy=%2212%22 r=%2210%22 fill=%22%23ffa800%22/%3E%3C/svg%3E'
    };

    function escapeHtml(value) {
      return String(value === null || value === undefined ? '' : value)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#039;');
    }

    function isRecord(value) {
      return value !== null && typeof value === 'object' && !Array.isArray(value);
    }

    function safeText(value, fallback) {
      if (typeof value === 'string') return value;
      if (typeof value === 'number' && Number.isFinite(value)) return String(value);
      return fallback || '';
    }

    function finiteNumber(value, fallback) {
      if (value === null || value === undefined || value === '' || typeof value === 'boolean') return fallback;
      const number = Number(value);
      return Number.isFinite(number) ? number : fallback;
    }

    function normalizeScores(value) {
      if (!Array.isArray(value)) return [];
      return value.slice(0, 16).map(item => finiteNumber(item, null));
    }

    function isBlockedDetailKey(key) {
      const lower = String(key).toLowerCase();
      return lower.includes('ch' + 'at') || lower.includes('mes' + 'sage') || lower.includes('com' + 'ment');
    }

    function sanitizeDetailValue(value, depth) {
      const level = depth || 0;
      if (level > 8) return null;
      if (Array.isArray(value)) return value.slice(0, 250).map(item => sanitizeDetailValue(item, level + 1));
      if (isRecord(value)) {
        const cleaned = {};
        Object.entries(value).forEach(([key, item]) => {
          if (!isBlockedDetailKey(key)) cleaned[key] = sanitizeDetailValue(item, level + 1);
        });
        return cleaned;
      }
      if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
      return typeof value === 'number' && Number.isFinite(value) ? value : null;
    }

    function extractStatusLabel(value) {
      if (typeof value === 'string' && value.trim() && !Number.isFinite(Number(value.trim()))) return value.trim();
      if (!isRecord(value)) return '';
      for (const key of ['label', 'name', 'text', 'shortName', 'statusName']) {
        const label = safeText(value[key], '').trim();
        if (label) return label;
      }
      return '';
    }

    function rawStatusLabel(rawPayload) {
      if (!isRecord(rawPayload)) return '';
      const candidates = [
        rawPayload.status_label, rawPayload.statusLabel, rawPayload.status_text,
        rawPayload.statusText, rawPayload.statusName, rawPayload.matchStatus,
        rawPayload.status, rawPayload.state
      ];
      for (const candidate of candidates) {
        const label = extractStatusLabel(candidate);
        if (label) return label;
      }
      return '';
    }

    function normalizeMatch(raw) {
      if (!isRecord(raw)) return null;
      const id = safeText(raw.id, '').trim();
      const sportId = finiteNumber(raw.sport_id, null);
      if (!id || ![1, 2].includes(sportId)) return null;
      const rawPayload = isRecord(raw.raw_payload) ? sanitizeDetailValue(raw.raw_payload) : {};
      return {
        id,
        sport_id: sportId,
        home_team_id: safeText(raw.home_team_id, ''),
        away_team_id: safeText(raw.away_team_id, ''),
        match_time: finiteNumber(raw.match_time, 0),
        status_id: finiteNumber(raw.status_id, null),
        status_label: rawStatusLabel(rawPayload),
        home_scores: normalizeScores(raw.home_scores),
        away_scores: normalizeScores(raw.away_scores),
        home_name: safeText(raw.home_name, 'Team A'),
        away_name: safeText(raw.away_name, 'Team B'),
        comp_name: safeText(raw.comp_name, 'Other League'),
        home_logo: typeof raw.home_logo === 'string' ? raw.home_logo : '',
        away_logo: typeof raw.away_logo === 'string' ? raw.away_logo : '',
        comp_logo: typeof raw.comp_logo === 'string' ? raw.comp_logo : '',
        raw_payload: rawPayload,
        version: safeText(raw.updated_at, '') + '|' + safeText(raw.detail_updated_at, '')
      };
    }

    function normalizeMatchesPayload(payload) {
      if (!Array.isArray(payload)) throw new Error('Live matches response must be an array');
      return payload
        .map(normalizeMatch)
        .filter(Boolean)
        .sort((a, b) => a.match_time - b.match_time || a.id.localeCompare(b.id));
    }

    function resolveAssetUrl(value, sportId, kind) {
      const fallback = kind === 'team' ? DEFAULT_LOGOS[sportId] : '';
      if (typeof value !== 'string' || !value.trim()) return fallback;
      const trimmed = value.trim();
      if (/^[a-z][a-z0-9+.-]*:/i.test(trimmed)) {
        try {
          const parsed = new URL(trimmed);
          return parsed.protocol === 'https:' ? parsed.href : fallback;
        } catch (_) {
          return fallback;
        }
      }
      let relative = trimmed;
      while (relative.startsWith('/')) relative = relative.slice(1);
      if (!relative || relative.split('/').includes('..')) return fallback;
      const encodedPath = relative.split('/').filter(Boolean).map(encodeURIComponent).join('/');
      if (!encodedPath) return fallback;
      const sportPath = sportId === 2 ? 'basketball' : 'football';
      return 'https://img.aiscore.com/' + sportPath + '/' + kind + '/' + encodedPath;
    }

    function renderImage(src, className, fallback) {
      if (!src) return '';
      return '<img class="' + className + '" src="' + escapeHtml(src) + '" alt="" data-fallback="' + escapeHtml(fallback || '') + '">';
    }

    function getStatusInfo(sportId, statusId, rawLabel) {
      let mapping = {};
      if (sportId === 1) {
        mapping = {
          1: { label: 'Scheduled', state: 'upcoming' },
          2: { label: 'First Half', state: 'live' },
          3: { label: 'HT', state: 'live' },
          4: { label: 'Second Half', state: 'live' },
          5: { label: 'Overtime', state: 'live' },
          6: { label: 'Penalty', state: 'live' },
          7: { label: 'Finished', state: 'finished' },
          8: { label: 'FT', state: 'finished' },
          9: { label: 'Postponed', state: 'finished' },
          10: { label: 'Cancelled', state: 'finished' },
          11: { label: 'Abandoned', state: 'finished' }
        };
      } else if (sportId === 2) {
        mapping = {
          1: { label: 'Scheduled', state: 'upcoming' },
          2: { label: 'Q1', state: 'live' },
          3: { label: 'Q1 Inter.', state: 'live' },
          4: { label: 'Q2', state: 'live' },
          5: { label: 'Q2 Inter.', state: 'live' },
          6: { label: 'Q3', state: 'live' },
          7: { label: 'Q3 Inter.', state: 'live' },
          8: { label: 'Q4', state: 'live' },
          9: { label: 'OT', state: 'live' },
          10: { label: 'Finished', state: 'finished' },
          11: { label: 'FT', state: 'finished' },
          12: { label: 'Postponed', state: 'finished' },
          13: { label: 'Cancelled', state: 'finished' },
          14: { label: 'Abandoned', state: 'finished' }
        };
      }
      if (mapping[statusId]) return { label: mapping[statusId].label, state: mapping[statusId].state, known: true };
      return { label: rawLabel || 'Unknown status', state: 'unknown', known: false };
    }

    function liveClock(match) {
      if (!isRecord(match.raw_payload)) return '';
      const candidates = match.sport_id === 1
        ? [match.raw_payload.minute, match.raw_payload.matchMinute, match.raw_payload.clock]
        : [match.raw_payload.clock, match.raw_payload.matchClock, match.raw_payload.remainingTime];
      for (const value of candidates) {
        if (typeof value === 'number' && Number.isFinite(value)) return match.sport_id === 1 ? value + String.fromCharCode(39) : String(value);
        if (typeof value === 'string' && value.trim()) return value.trim();
      }
      return '';
    }

    function scoreAt(scores, index) {
      const value = scores[index];
      return typeof value === 'number' && Number.isFinite(value) ? value : null;
    }

    function displayScore(value) {
      return value === null ? '-' : String(value);
    }

    function formatMatchTime(timestamp) {
      const date = new Date(timestamp * 1000);
      if (!timestamp || Number.isNaN(date.getTime())) return 'Time unavailable';
      return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    }

    function renderMatchCard(match, preserved) {
      const statusInfo = getStatusInfo(match.sport_id, match.status_id, match.status_label);
      const clock = statusInfo.state === 'live' ? liveClock(match) : '';
      const statusText = statusInfo.label + (clock && !statusInfo.label.includes(clock) ? ' · ' + clock : '');
      const statusClass = statusInfo.state === 'live' ? 'status-live'
        : statusInfo.state === 'finished' ? 'status-ft'
        : statusInfo.state === 'unknown' ? 'status-unknown' : 'status-upcoming';
      const homeTotal = scoreAt(match.home_scores, 0);
      const awayTotal = scoreAt(match.away_scores, 0);
      let breakdownHtml = '';
      let homeBadges = '';
      let awayBadges = '';

      if (match.sport_id === 1) {
        const homeRed = scoreAt(match.home_scores, 2);
        const awayRed = scoreAt(match.away_scores, 2);
        const homeYellow = scoreAt(match.home_scores, 3);
        const awayYellow = scoreAt(match.away_scores, 3);
        if (homeRed > 0) homeBadges += '<span class="red-card-badge">' + homeRed + '</span>';
        if (awayRed > 0) awayBadges += '<span class="red-card-badge">' + awayRed + '</span>';
        if (homeYellow > 0) homeBadges += '<span class="yellow-card-badge">' + homeYellow + '</span>';
        if (awayYellow > 0) awayBadges += '<span class="yellow-card-badge">' + awayYellow + '</span>';
        const homeHalf = scoreAt(match.home_scores, 1);
        const awayHalf = scoreAt(match.away_scores, 1);
        if (homeHalf !== null || awayHalf !== null) breakdownHtml += '<span class="score-half">HT ' + displayScore(homeHalf) + ' - ' + displayScore(awayHalf) + '</span>';
        const homeCorners = scoreAt(match.home_scores, 4);
        const awayCorners = scoreAt(match.away_scores, 4);
        if (homeCorners !== null || awayCorners !== null) breakdownHtml += '<div class="corners-info">🚩 ' + displayScore(homeCorners) + ' - ' + displayScore(awayCorners) + '</div>';
      } else {
        const periods = [];
        const periodCount = Math.max(match.home_scores.length, match.away_scores.length);
        for (let index = 1; index < Math.min(periodCount, 9); index++) {
          const homePeriod = scoreAt(match.home_scores, index);
          const awayPeriod = scoreAt(match.away_scores, index);
          if (homePeriod === null && awayPeriod === null) continue;
          const label = index <= 4 ? 'Q' + index : 'OT' + (index - 4);
          periods.push(label + ': ' + displayScore(homePeriod) + '-' + displayScore(awayPeriod));
        }
        if (periods.length) breakdownHtml += '<div class="basketball-quarters">' + periods.join(' | ') + '</div>';
      }

      const isExpanded = preserved.expandedId === match.id;
      const sameVersion = isExpanded && preserved.version === match.version;
      const detailsState = sameVersion ? preserved.detailsState : 'false';
      const detailsHtml = sameVersion ? preserved.detailsHtml : '';
      const homeLogo = resolveAssetUrl(match.home_logo, match.sport_id, 'team');
      const awayLogo = resolveAssetUrl(match.away_logo, match.sport_id, 'team');
      const competitionLogo = resolveAssetUrl(match.comp_logo, match.sport_id, 'competition');

      return '<div class="match-card ' + (isExpanded ? 'expanded' : '') + '"' +
        ' data-match-id="' + escapeHtml(match.id) + '" data-home-id="' + escapeHtml(match.home_team_id) + '"' +
        ' data-away-id="' + escapeHtml(match.away_team_id) + '" data-version="' + escapeHtml(match.version) + '"' +
        ' tabindex="0" role="button" aria-expanded="' + (isExpanded ? 'true' : 'false') + '">' +
          '<div class="match-header"><div class="league-info">' + renderImage(competitionLogo, 'league-logo', '') +
            '<span>' + escapeHtml(match.comp_name) + '</span></div><span class="match-status ' + statusClass + '">' + escapeHtml(statusText) + '</span></div>' +
          '<div class="match-body"><div class="team team-home"><span class="team-name">' + escapeHtml(match.home_name) + ' ' + homeBadges + '</span>' +
            renderImage(homeLogo, 'team-logo', DEFAULT_LOGOS[match.sport_id]) + '</div>' +
            '<div class="score-area"><span class="score-live">' + displayScore(homeTotal) + ' - ' + displayScore(awayTotal) + '</span>' + breakdownHtml +
              '<span class="' + (statusInfo.state === 'upcoming' ? 'match-time' : 'score-half') + '">' + escapeHtml(formatMatchTime(match.match_time)) + '</span></div>' +
            '<div class="team team-away">' + renderImage(awayLogo, 'team-logo', DEFAULT_LOGOS[match.sport_id]) +
              '<span class="team-name">' + escapeHtml(match.away_name) + ' ' + awayBadges + '</span></div></div>' +
          '<div class="match-details-container" data-loaded="' + escapeHtml(detailsState) + '">' + detailsHtml + '</div></div>';
    }

    function captureRenderState() {
      const expanded = document.querySelector('.match-card.expanded');
      const details = expanded ? expanded.querySelector('.match-details-container') : null;
      const focusedCard = document.activeElement ? document.activeElement.closest('.match-card') : null;
      return {
        expandedId: expanded ? expanded.dataset.matchId : '',
        version: expanded ? expanded.dataset.version : '',
        detailsHtml: details ? details.innerHTML : '',
        detailsState: details ? details.dataset.loaded : 'false',
        focusedId: focusedCard ? focusedCard.dataset.matchId : '',
        footballScroll: document.getElementById('football-matches').scrollTop,
        basketballScroll: document.getElementById('basketball-matches').scrollTop,
        windowScroll: window.scrollY
      };
    }

    function renderMatches(matches) {
      const preserved = captureRenderState();
      const football = matches.filter(match => match.sport_id === 1);
      const basketball = matches.filter(match => match.sport_id === 2);
      const footballList = document.getElementById('football-matches');
      const basketballList = document.getElementById('basketball-matches');
      footballList.innerHTML = football.length ? football.map(match => renderMatchCard(match, preserved)).join('') : '<div class="empty-state">No live or today football matches.</div>';
      basketballList.innerHTML = basketball.length ? basketball.map(match => renderMatchCard(match, preserved)).join('') : '<div class="empty-state">No live or today basketball matches.</div>';
      document.getElementById('football-count').textContent = football.length + ' matches';
      document.getElementById('basketball-count').textContent = basketball.length + ' matches';
      footballList.scrollTop = preserved.footballScroll;
      basketballList.scrollTop = preserved.basketballScroll;
      if (preserved.focusedId) {
        const focused = Array.from(document.querySelectorAll('.match-card')).find(card => card.dataset.matchId === preserved.focusedId);
        if (focused) focused.focus({ preventScroll: true });
      }
      requestAnimationFrame(() => window.scrollTo({ top: preserved.windowScroll, behavior: 'auto' }));
      const expanded = document.querySelector('.match-card.expanded');
      if (expanded && expanded.querySelector('.match-details-container').dataset.loaded !== 'true') toggleCard(expanded, true);
    }

    function setRefreshStatus(message, state) {
      const info = document.getElementById('sync-info');
      info.textContent = message;
      info.className = state ? 'refresh-' + state : '';
    }

    let matchesInFlight = false;
    let activeAbortController = null;
    let pollTimer = null;
    let consecutiveFailures = 0;
    let lastSuccessfulRefresh = null;

    function scheduleNextPoll(delay) {
      clearTimeout(pollTimer);
      pollTimer = null;
      if (!document.hidden) pollTimer = setTimeout(loadMatches, delay);
    }

    async function loadMatches() {
      if (matchesInFlight) return;
      matchesInFlight = true;
      activeAbortController = new AbortController();
      let succeeded = false;
      try {
        const response = await fetch('/api/matches/live', { signal: activeAbortController.signal });
        if (!response.ok) throw new Error('Live matches request failed (' + response.status + ')');
        renderMatches(normalizeMatchesPayload(await response.json()));
        lastSuccessfulRefresh = new Date();
        consecutiveFailures = 0;
        succeeded = true;
        setRefreshStatus('Last successful refresh: ' + lastSuccessfulRefresh.toLocaleTimeString(), '');
      } catch (err) {
        if (err.name !== 'AbortError') {
          consecutiveFailures += 1;
          const retryMs = Math.min(BASE_POLL_MS * Math.pow(2, consecutiveFailures), MAX_POLL_MS);
          if (lastSuccessfulRefresh) {
            setRefreshStatus('Refresh failed; showing data from ' + lastSuccessfulRefresh.toLocaleTimeString() + '. Retrying in ' + Math.round(retryMs / 1000) + 's.', 'stale');
          } else {
            setRefreshStatus('Unable to load matches. Retrying in ' + Math.round(retryMs / 1000) + 's.', 'error');
            document.getElementById('football-matches').innerHTML = '<div class="empty-state">Unable to load matches. Automatic retry is active.</div>';
            document.getElementById('basketball-matches').innerHTML = '<div class="empty-state">Unable to load matches. Automatic retry is active.</div>';
          }
          console.error('Error fetching matches:', err);
        }
      } finally {
        matchesInFlight = false;
        activeAbortController = null;
        scheduleNextPoll(succeeded ? BASE_POLL_MS : Math.min(BASE_POLL_MS * Math.pow(2, consecutiveFailures), MAX_POLL_MS));
      }
    }

    function isMeaningful(value) {
      if (value === null || value === undefined || value === '') return false;
      if (Array.isArray(value)) return value.some(isMeaningful);
      if (isRecord(value)) return Object.values(value).some(isMeaningful);
      return true;
    }

    function humanizeKey(key) {
      return String(key).replace(/([a-z0-9])([A-Z])/g, '$1 $2').replace(/[_-]+/g, ' ').replace(/^./, char => char.toUpperCase());
    }

    function renderStructuredValue(value, depth) {
      const level = depth || 0;
      if (level > 5) return '<span>Additional nested data</span>';
      if (Array.isArray(value)) {
        if (!value.length) return '<span>Not available</span>';
        const items = value.slice(0, 50).map(item => '<li>' + renderStructuredValue(item, level + 1) + '</li>').join('');
        return '<ol class="detail-list">' + items + (value.length > 50 ? '<li>More items omitted</li>' : '') + '</ol>';
      }
      if (isRecord(value)) {
        const entries = Object.entries(value).filter(([key, item]) => !isBlockedDetailKey(key) && isMeaningful(item)).slice(0, 80);
        if (!entries.length) return '<span>Not available</span>';
        return '<dl class="detail-grid">' + entries.map(([key, item]) => '<dt>' + escapeHtml(humanizeKey(key)) + '</dt><dd>' + renderStructuredValue(item, level + 1) + '</dd>').join('') + '</dl>';
      }
      if (typeof value === 'boolean') return value ? 'Yes' : 'No';
      return '<span>' + escapeHtml(value) + '</span>';
    }

    function renderDetailSection(icon, title, value) {
      if (!isMeaningful(value)) return '';
      return '<div class="detail-section-title">' + icon + ' ' + escapeHtml(title) + '</div>' + renderStructuredValue(value, 0);
    }

    function normalizeDetailPayload(payload) {
      const safe = sanitizeDetailValue(payload);
      if (!isRecord(safe)) throw new Error('Malformed match detail response');
      const rawPayload = isRecord(safe.raw_payload) ? safe.raw_payload : {};
      const reserved = new Set(['match_id', 'sport_id', 'incidents', 'stats', 'lineups', 'odds', 'h2h', 'raw_payload', 'extra', 'last_updated']);
      const extra = {};
      Object.entries(rawPayload).forEach(([key, value]) => {
        if (!reserved.has(key) && !isBlockedDetailKey(key)) extra[key] = value;
      });
      if (isRecord(safe.extra)) Object.assign(extra, safe.extra);
      Object.entries(safe).forEach(([key, value]) => {
        if (!reserved.has(key) && !isBlockedDetailKey(key)) extra[key] = value;
      });
      return {
        incidents: Array.isArray(safe.incidents) ? safe.incidents : [],
        stats: isRecord(safe.stats) || Array.isArray(safe.stats) ? safe.stats : {},
        lineups: isRecord(safe.lineups) || Array.isArray(safe.lineups) ? safe.lineups : {},
        odds: isRecord(safe.odds) || Array.isArray(safe.odds) ? safe.odds : {},
        h2h: isRecord(safe.h2h) || Array.isArray(safe.h2h) ? safe.h2h : {},
        extra
      };
    }

    function renderDetails(payload) {
      const detail = normalizeDetailPayload(payload);
      return renderDetailSection('⏱️', 'Timeline / Incidents', detail.incidents) +
        renderDetailSection('📊', 'Match Stats', detail.stats) +
        renderDetailSection('📋', 'Lineups', detail.lineups) +
        renderDetailSection('📈', 'Odds', detail.odds) +
        renderDetailSection('⚔️', 'Head-to-Head', detail.h2h) +
        renderDetailSection('➕', 'Additional Data', detail.extra);
    }

    async function responseError(response, fallback) {
      try {
        const body = await response.json();
        return safeText(body.error, fallback);
      } catch (_) {
        return fallback;
      }
    }

    async function toggleCard(card, forceFetch) {
      const force = forceFetch === true;
      const wasExpanded = card.classList.contains('expanded');
      document.querySelectorAll('.match-card.expanded').forEach(other => {
        if (other !== card) {
          other.classList.remove('expanded');
          other.setAttribute('aria-expanded', 'false');
        }
      });
      if (wasExpanded && !force) {
        card.classList.remove('expanded');
        card.setAttribute('aria-expanded', 'false');
        return;
      }
      card.classList.add('expanded');
      card.setAttribute('aria-expanded', 'true');
      const detailsContainer = card.querySelector('.match-details-container');
      if ((detailsContainer.dataset.loaded === 'true' && !force) || detailsContainer.dataset.loading === 'true') return;
      detailsContainer.dataset.loading = 'true';
      detailsContainer.innerHTML = '<div class="loading-spinner">⏳ Loading available match details...</div>';
      try {
        const response = await fetch('/api/matches/detail?match_id=' + encodeURIComponent(card.dataset.matchId));
        if (response.status === 404) {
          detailsContainer.innerHTML = '<div class="empty-state" style="font-size: 0.8rem; padding: 12px;">Details are pending or not available yet.</div>';
          detailsContainer.dataset.loaded = 'pending';
          return;
        }
        if (!response.ok) throw new Error(await responseError(response, 'Unable to load match details'));
        const html = renderDetails(await response.json());
        if (html) {
          detailsContainer.innerHTML = html;
          detailsContainer.dataset.loaded = 'true';
        } else {
          detailsContainer.innerHTML = '<div class="empty-state" style="font-size: 0.8rem; padding: 12px;">Details are pending or not available yet.</div>';
          detailsContainer.dataset.loaded = 'pending';
        }
      } catch (err) {
        detailsContainer.dataset.loaded = 'false';
        detailsContainer.innerHTML = '<div class="empty-state" style="color: var(--accent); font-size: 0.8rem; padding: 12px;">⚠️ ' + escapeHtml(err.message || 'Unable to load match details') + '</div>';
      } finally {
        delete detailsContainer.dataset.loading;
      }
    }

    document.addEventListener('error', event => {
      if (!(event.target instanceof HTMLImageElement)) return;
      const fallback = event.target.dataset.fallback;
      if (fallback && event.target.dataset.fallbackUsed !== 'true') {
        event.target.dataset.fallbackUsed = 'true';
        event.target.src = fallback;
      } else {
        event.target.hidden = true;
      }
    }, true);

    document.addEventListener('click', event => {
      const card = event.target.closest('.match-card');
      if (card && !event.target.closest('#settings-modal') && !event.target.closest('.settings-btn')) toggleCard(card, false);
    });

    document.addEventListener('keydown', event => {
      const card = event.target.closest('.match-card');
      if (card && (event.key === 'Enter' || event.key === ' ')) {
        event.preventDefault();
        toggleCard(card, false);
      }
    });

    const settingsModal = document.getElementById('settings-modal');
    const alertBanner = document.getElementById('modal-alert');
    const tokenInput = document.getElementById('api-token');
    const saveSettingsButton = document.getElementById('save-settings');

    function showBanner(message, type) {
      alertBanner.textContent = message;
      alertBanner.className = 'alert-banner alert-' + type;
      alertBanner.style.display = 'block';
    }

    function clearBanner() {
      alertBanner.textContent = '';
      alertBanner.style.display = 'none';
    }

    document.getElementById('open-settings').onclick = async () => {
      clearBanner();
      tokenInput.value = '';
      settingsModal.style.display = 'flex';
      document.getElementById('sync-interval').focus();
      try {
        const response = await fetch('/api/settings');
        if (!response.ok) throw new Error(await responseError(response, 'Unable to load settings'));
        const settings = await response.json();
        if (!isRecord(settings)) throw new Error('Malformed settings response');
        if (settings.sync_interval_mins !== undefined) document.getElementById('sync-interval').value = settings.sync_interval_mins;
        if (settings.detail_update_interval_secs !== undefined) document.getElementById('detail-interval').value = settings.detail_update_interval_secs;
      } catch (err) {
        showBanner(err.message || 'Unable to load settings', 'error');
      }
    };

    document.getElementById('close-settings').onclick = () => {
      tokenInput.value = '';
      settingsModal.style.display = 'none';
      document.getElementById('open-settings').focus();
    };

    document.getElementById('save-settings').onclick = async () => {
      clearBanner();
      const token = tokenInput.value.trim();
      const sync = Number(document.getElementById('sync-interval').value);
      const detail = Number(document.getElementById('detail-interval').value);
      if (!Number.isInteger(sync) || sync < 1 || sync > 60) {
        showBanner('D1 sync interval must be a whole number from 1 to 60 minutes.', 'error');
        return;
      }
      if (!Number.isInteger(detail) || (detail !== 0 && (detail < 5 || detail > 3600))) {
        showBanner('Detail update interval must be 0 or between 5 and 3600 seconds.', 'error');
        return;
      }
      if (!token) {
        showBanner('Enter the authorization API token to save settings.', 'error');
        return;
      }
      saveSettingsButton.disabled = true;
      try {
        const response = await fetch('/api/settings', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', 'Authorization': 'Bearer ' + token },
          body: JSON.stringify({ sync_interval_mins: sync, detail_update_interval_secs: detail })
        });
        if (!response.ok) throw new Error(await responseError(response, 'Unable to save settings'));
        showBanner('Settings saved. The new values control SQLite-to-D1 sync and crawler detail updates.', 'success');
      } catch (err) {
        showBanner(err.message || 'Network error while saving settings', 'error');
      } finally {
        tokenInput.value = '';
        saveSettingsButton.disabled = false;
      }
    };

    document.addEventListener('keydown', event => {
      if (event.key === 'Escape' && settingsModal.style.display === 'flex') document.getElementById('close-settings').click();
    });

    document.addEventListener('visibilitychange', () => {
      if (document.hidden) {
        clearTimeout(pollTimer);
        pollTimer = null;
      } else if (!matchesInFlight) {
        loadMatches();
      }
    });

    window.addEventListener('pagehide', () => {
      clearTimeout(pollTimer);
      if (activeAbortController) activeAbortController.abort();
    });

    loadMatches();
  </script>
</body>
</html>`;

      return new Response(html, {
        headers: { "Content-Type": "text/html; charset=UTF-8" }
      });
    }

    return new Response("Not Found", { status: 404 });
  }
};
