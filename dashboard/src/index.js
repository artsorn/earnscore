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
                "INSERT INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, raw_payload, synced, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, datetime('now')) ON CONFLICT(id) DO UPDATE SET status_id=excluded.status_id, home_scores=excluded.home_scores, away_scores=excluded.away_scores, raw_payload=excluded.raw_payload, synced=1, updated_at=datetime('now')"
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
        let query = `
          SELECT 
            m.id, m.sport_id, m.home_team_id, m.away_team_id, m.match_time, m.status_id, m.home_scores, m.away_scores, m.updated_at, m.raw_payload,
            ht.name as home_name, ht.logo as home_logo, ht.slug as home_slug,
            at.name as away_name, at.logo as away_logo, at.slug as away_slug,
            c.name as comp_name, c.logo as comp_logo, c.country_name, c.country_logo
          FROM matches m
          LEFT JOIN teams ht ON m.home_team_id = ht.id
          LEFT JOIN teams at ON m.away_team_id = at.id
          LEFT JOIN competitions c ON m.competition_id = c.id
        `;
        const params = [];
        if (sportFilter) {
          query += " WHERE m.sport_id = ?1";
          params.push(parseInt(sportFilter, 10));
        }
        query += " ORDER BY m.match_time ASC";

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
          "SELECT match_id, sport_id, incidents, stats, lineups, odds, h2h, last_updated FROM match_details WHERE match_id = ?1"
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

        const formatted = {
          ...result,
          incidents: safeParse(result.incidents, "[]"),
          stats: safeParse(result.stats, "{}"),
          lineups: safeParse(result.lineups, "{}"),
          odds: safeParse(result.odds, "{}"),
          h2h: safeParse(result.h2h, "{}")
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
            const val = Math.min(Math.max(parseInt(sync_interval_mins, 10) || 5, 1), 60);
            statements.push(
              env.DB.prepare("INSERT OR REPLACE INTO settings (key, value) VALUES ('sync_interval_mins', ?1)").bind(String(val))
            );
          }
          if (detail_update_interval_secs !== undefined) {
            const val = Math.min(Math.max(parseInt(detail_update_interval_secs, 10) || 60, 5), 3600);
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

    .nav-tabs {
      display: flex;
      gap: 15px;
    }

    .tab-btn {
      background: var(--card);
      border: 1px solid #322b5e;
      color: var(--text-dim);
      padding: 8px 18px;
      border-radius: 20px;
      cursor: pointer;
      font-weight: 600;
      transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
      display: flex;
      align-items: center;
      gap: 8px;
    }

    .tab-btn:hover {
      color: var(--text);
      border-color: var(--primary-light);
      transform: translateY(-2px);
    }

    .tab-btn.active {
      background: linear-gradient(135deg, var(--primary) 0%, var(--primary-light) 100%);
      color: white;
      border-color: transparent;
      box-shadow: 0 5px 15px rgba(138, 43, 226, 0.4);
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
    }

    .match-card:hover {
      border-color: var(--primary-light);
      transform: scale(1.01);
      box-shadow: 0 5px 15px rgba(0, 0, 0, 0.4);
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

    /* Settings Panel */
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

    @keyframes pulse {
      0% { opacity: 0.8; transform: scale(1); }
      50% { opacity: 1; transform: scale(1.02); }
      100% { opacity: 0.8; transform: scale(1); }
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
    
    .match-card {
      cursor: pointer;
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
      <div id="sync-info" style="font-size: 0.85rem; color: var(--text-dim); text-align: right;">
        Last sync: --:--
      </div>
      <button class="settings-btn" id="open-settings">⚙️</button>
    </div>
  </header>

  <main>
    <div class="sports-sections">
      <!-- Football Section -->
      <div class="section-card">
        <h2 class="section-title football-title">
          <span>⚽ Football Live</span>
          <span id="football-count" style="font-size: 1rem; color: var(--text-dim);">0 matches</span>
        </h2>
        <div class="match-list" id="football-matches">
          <div class="empty-state">Loading matches...</div>
        </div>
      </div>

      <!-- Basketball Section -->
      <div class="section-card">
        <h2 class="section-title basketball-title">
          <span>🏀 Basketball Live</span>
          <span id="basketball-count" style="font-size: 1rem; color: var(--text-dim);">0 matches</span>
        </h2>
        <div class="match-list" id="basketball-matches">
          <div class="empty-state">Loading matches...</div>
        </div>
      </div>
    </div>
  </main>

  <!-- Settings Modal -->
  <div class="modal" id="settings-modal">
    <div class="modal-content">
      <h3 class="modal-title">⚙️ System Settings</h3>
      <div class="form-group">
        <label>Sync Interval (every X minutes)</label>
        <input type="number" id="sync-interval" min="1" max="60" value="5">
      </div>
      <div class="form-group">
        <label>Match Details Update Interval</label>
        <select id="detail-interval">
          <option value="60">1 Minute (60s)</option>
          <option value="120">2 Minutes</option>
          <option value="240">4 Minutes</option>
          <option value="480">8 Minutes</option>
          <option value="960">16 Minutes</option>
          <option value="0">First & Last Time Only (No Live Updates)</option>
        </select>
      </div>
      <div class="form-group">
        <label>Change Security API Token</label>
        <input type="password" id="api-token" placeholder="••••••••">
      </div>
      <div class="modal-actions">
        <button class="btn btn-cancel" id="close-settings">Cancel</button>
        <button class="btn btn-save" id="save-settings">Save Settings</button>
      </div>
    </div>
  </div>

  <script>
    // Fetch live matches from Workers DB
    async function loadMatches() {
      try {
        const response = await fetch('/api/matches/live');
        if (!response.ok) throw new Error('API error');
        const matches = await response.json();

        const fList = document.getElementById('football-matches');
        const bList = document.getElementById('basketball-matches');
        
        let fHtml = '';
        let bHtml = '';
        let fCount = 0;
        let bCount = 0;

        // Keep track of which card was expanded to preserve state during refresh
        const expandedCardId = document.querySelector('.match-card.expanded')?.dataset.matchId;

        matches.forEach(m => {
          const homeScore = m.home_scores[0] !== undefined ? m.home_scores[0] : '-';
          const awayScore = m.away_scores[0] !== undefined ? m.away_scores[0] : '-';
          const isLive = m.status_id > 1 && m.status_id < 8; // Status 1 is Scheduled, 8 is FT
          const isFT = m.status_id === 8;
          
          let statusText = 'Upcoming';
          let statusClass = 'status-upcoming';
          
          if (isLive) {
            statusText = m.status_id === 3 ? 'HT' : 'Live';
            statusClass = 'status-live';
          } else if (isFT) {
            statusText = 'FT';
            statusClass = 'status-ft';
          }

          const matchDate = new Date(m.match_time * 1000);
          const timeStr = matchDate.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

          // Detailed scores & indicators
          let redCardsHomeHtml = '';
          let redCardsAwayHtml = '';
          let yellowCardsHomeHtml = '';
          let yellowCardsAwayHtml = '';
          let cornersHtml = '';
          let halfTimeHtml = '';
          let basketballQuartersHtml = '';

          if (m.sport_id === 1) { // Football
            // Red cards (Index 2)
            if (m.home_scores[2] > 0) redCardsHomeHtml = '<span class="red-card-badge">' + m.home_scores[2] + '</span>';
            if (m.away_scores[2] > 0) redCardsAwayHtml = '<span class="red-card-badge">' + m.away_scores[2] + '</span>';
            
            // Yellow cards (Index 3)
            if (m.home_scores[3] > 0) yellowCardsHomeHtml = '<span class="yellow-card-badge">' + m.home_scores[3] + '</span>';
            if (m.away_scores[3] > 0) yellowCardsAwayHtml = '<span class="yellow-card-badge">' + m.away_scores[3] + '</span>';
            
            // Corners (Index 4)
            if (m.home_scores[4] !== undefined && m.away_scores[4] !== undefined) {
              cornersHtml = '<div class="corners-info">🚩 ' + m.home_scores[4] + ' - ' + m.away_scores[4] + '</div>';
            }
            
            // Half-time score (Index 1)
            if (m.home_scores[1] !== undefined && m.away_scores[1] !== undefined && (isLive || isFT)) {
              halfTimeHtml = '<span class="score-half">(HT ' + m.home_scores[1] + ' - ' + m.away_scores[1] + ')</span>';
            }
          } else if (m.sport_id === 2) { // Basketball
            // Quarters scores
            const quarters = [];
            for (let i = 1; i <= 4; i++) {
              if (m.home_scores[i] !== undefined || m.away_scores[i] !== undefined) {
                quarters.push('Q' + i + ': ' + (m.home_scores[i] || 0) + '-' + (m.away_scores[i] || 0));
              }
            }
            if (quarters.length > 0) {
              basketballQuartersHtml = '<div class="basketball-quarters">' + quarters.join(' | ') + '</div>';
            }
          }

          const isExpanded = m.id == expandedCardId;

          const cardHtml = \`
            <div class="match-card \${isExpanded ? 'expanded' : ''}" data-match-id="\${m.id}" data-home-id="\${m.home_team_id}" data-away-id="\${m.away_team_id}">
              <div class="match-header">
                <div class="league-info">
                  \${m.comp_logo ? \`<img class="league-logo" src="https://img.aiscore.com/football/competition/\${m.comp_logo}" alt="">\` : ''}
                  <span>\${m.comp_name || 'Other League'}</span>
                </div>
                <span class="match-status \${statusClass}">\${statusText}</span>
              </div>
              <div class="match-body">
                <div class="team team-home">
                  <span class="team-name">
                    \${m.home_name || 'Team A'}
                    \${redCardsHomeHtml}
                    \${yellowCardsHomeHtml}
                  </span>
                  \${m.home_logo ? \`<img class="team-logo" src="https://img.aiscore.com/football/team/\${m.home_logo}" alt="">\` : ''}
                </div>
                <div class="score-area">
                  <span class="score-live">\${homeScore} - \${awayScore}</span>
                  \${halfTimeHtml}
                  \${cornersHtml}
                  \${basketballQuartersHtml}
                  \${isLive || isFT ? \`<span class="score-half">Time: \${timeStr}</span>\` : \`<span class="match-time">\${timeStr}</span>\`}
                </div>
                <div class="team team-away">
                  \${m.away_logo ? \`<img class="team-logo" src="https://img.aiscore.com/football/team/\${m.away_logo}" alt="">\` : ''}
                  <span class="team-name">
                    \${m.away_name || 'Team B'}
                    \${redCardsAwayHtml}
                    \${yellowCardsAwayHtml}
                  </span>
                </div>
              </div>
              <div class="match-details-container" data-loaded="\${isExpanded ? 'true' : 'false'}">
                \${isExpanded ? document.querySelector('.match-card.expanded .match-details-container')?.innerHTML || '' : ''}
              </div>
            </div>
          \`;

          if (m.sport_id === 1) {
            fHtml += cardHtml;
            fCount++;
          } else if (m.sport_id === 2) {
            bHtml += cardHtml;
            bCount++;
          }
        });

        fList.innerHTML = fHtml || '<div class="empty-state">No live football matches.</div>';
        bList.innerHTML = bHtml || '<div class="empty-state">No live basketball matches.</div>';
        
        document.getElementById('football-count').innerText = \`\${fCount} matches\`;
        document.getElementById('basketball-count').innerText = \`\${bCount} matches\`;

        // Update last sync time
        const now = new Date();
        document.getElementById('sync-info').innerText = 'Last sync: ' + now.toLocaleTimeString();
      } catch (err) {
        console.error('Error fetching matches:', err);
      }
    }

    // Click handler to expand card and lazy-load details
    document.addEventListener('click', async (e) => {
      const card = e.target.closest('.match-card');
      if (!card) return;
      
      // If clicking inside forms or settings, ignore
      if (e.target.closest('#settings-modal') || e.target.closest('.settings-btn')) return;

      const wasExpanded = card.classList.contains('expanded');
      
      // Collapse all other cards
      document.querySelectorAll('.match-card.expanded').forEach(c => {
        if (c !== card) c.classList.remove('expanded');
      });

      if (!wasExpanded) {
        card.classList.add('expanded');
        const matchId = card.dataset.matchId;
        const detailsContainer = card.querySelector('.match-details-container');
        
        if (detailsContainer.dataset.loaded === 'true') return;

        detailsContainer.innerHTML = '<div class="loading-spinner">⏳ Loading statistics & timeline...</div>';

        try {
          const res = await fetch('/api/matches/detail?match_id=' + matchId);
          if (!res.ok) throw new Error('Details not found in D1 database');
          const data = await res.json();
          
          let detailsHtml = '';

          // 1. Stats Section
          if (data.stats && Object.keys(data.stats).length > 0) {
            detailsHtml += '<div class="detail-section-title">📊 Match Stats</div><div class="stats-grid">';
            const statsMapping = {
              "possession": "Possession",
              "shotsOnTarget": "Shots on Target",
              "shotsOffTarget": "Shots off Target",
              "fouls": "Fouls",
              "corners": "Corners",
              "offsides": "Offsides",
              "yellowCards": "Yellow Cards",
              "redCards": "Red Cards"
            };
            for (const key of Object.keys(statsMapping)) {
              if (data.stats[key]) {
                const homeVal = data.stats[key].home || 0;
                const awayVal = data.stats[key].away || 0;
                const total = (Number(homeVal) + Number(awayVal)) || 1;
                const homePct = (Number(homeVal) / total) * 100;
                const awayPct = (Number(awayVal) / total) * 100;
                
                detailsHtml += 
                  '<div class="stat-row">' +
                    '<span style="text-align: right; font-weight: 600;">' + homeVal + '</span>' +
                    '<span style="color: var(--text-dim); font-size: 0.8rem;">' + statsMapping[key] + '</span>' +
                    '<span style="text-align: left; font-weight: 600;">' + awayVal + '</span>' +
                    '<div class="stat-bar-container">' +
                      '<div class="stat-bar-home" style="width: ' + homePct + '%"></div>' +
                      '<div class="stat-bar-away" style="width: ' + awayPct + '%"></div>' +
                    '</div>' +
                  '</div>';
              }
            }
            detailsHtml += '</div>';
          }

          // 2. Timeline Section (Incidents)
          if (data.incidents && data.incidents.length > 0) {
            detailsHtml += '<div class="detail-section-title">⏱️ Timeline</div><div class="timeline-list">';
            const sortedIncidents = [...data.incidents].sort((a, b) => Number(a.time) - Number(b.time));
            
            sortedIncidents.forEach(item => {
              let icon = '⚽';
              let text = '';
              const isHome = item.belong === 1;
              
              if (item.type === 1) {
                icon = '⚽';
                text = '<strong>Goal!</strong> ' + (item.player ? item.player.name : '') + ' - ' + item.homeScore + '-' + item.awayScore;
              } else if (item.type === 2) {
                icon = '🟥';
                text = '<strong>Red Card</strong> - ' + (item.player ? item.player.name : '');
              } else if (item.type === 3) {
                icon = '🟨';
                text = '<strong>Yellow Card</strong> - ' + (item.player ? item.player.name : '');
              } else if (item.type === 9) {
                icon = '🔄';
                text = '<strong>Sub</strong> - In: ' + (item.playerIn ? item.playerIn.name : '') + ' / Out: ' + (item.player ? item.player.name : '');
              } else {
                return;
              }

              detailsHtml += 
                '<div class="timeline-item" style="justify-content: ' + (isHome ? 'flex-start' : 'flex-end') + '; text-align: ' + (isHome ? 'left' : 'right') + '">' +
                  '<span class="timeline-time">' + item.time + '\\\'</span>' +
                  '<span class="timeline-icon">' + icon + '</span>' +
                  '<span>' + text + '</span>' +
                '</div>';
            });
            detailsHtml += '</div>';
          }

          // 3. Lineups Section
          if (data.lineups && data.lineups.lineup && (data.lineups.lineup.home || data.lineups.lineup.away)) {
            detailsHtml += '<div class="detail-section-title">📋 Lineups</div><div class="lineups-container">';
            const homePlayers = data.lineups.lineup.home || [];
            const awayPlayers = data.lineups.lineup.away || [];

            detailsHtml += '<div class="lineup-column"><div class="lineup-team-title">Home</div>';
            homePlayers.forEach(p => {
              detailsHtml += '<div class="player-row"><span>#' + (p.shirtNumber || '') + ' ' + (p.name || '') + '</span><span>' + (p.position || '') + '</span></div>';
            });
            detailsHtml += '</div>';

            detailsHtml += '<div class="lineup-column"><div class="lineup-team-title">Away</div>';
            awayPlayers.forEach(p => {
              detailsHtml += '<div class="player-row"><span>#' + (p.shirtNumber || '') + ' ' + (p.name || '') + '</span><span>' + (p.position || '') + '</span></div>';
            });
            detailsHtml += '</div>';
          }

          // 4. H2H Section
          if (data.h2h && data.h2h.h2h && data.h2h.h2h.length > 0) {
            detailsHtml += '<div class="detail-section-title">⚔️ Head-to-Head History</div><div style="display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; font-size: 0.75rem; color: var(--text-dim);">';
            const homeId = card.dataset.homeId;
            const homeName = card.querySelector('.team-home .team-name').innerText.split('\\n')[0].trim();
            const awayName = card.querySelector('.team-away .team-name').innerText.split('\\n')[0].trim();
            
            // Show up to 5 H2H matches
            const pastMatches = data.h2h.h2h.slice(0, 5);
            pastMatches.forEach(m => {
              const mDate = new Date(m.matchTime * 1000).toLocaleDateString();
              const scoreHome = m.homeScores ? m.homeScores[0] : 0;
              const scoreAway = m.awayScores ? m.awayScores[0] : 0;
              const isCurrentHome = m.homeTeam?.id == homeId;
              
              const hTeam = isCurrentHome ? homeName : awayName;
              const aTeam = isCurrentHome ? awayName : homeName;
              
              detailsHtml += 
                '<div style="display: grid; grid-template-columns: 2fr 3fr 1fr 3fr; padding: 4px 0; border-bottom: 1px solid #1f1b3a; text-align: center;">' +
                  '<span style="color: var(--text-dim); text-align: left;">' + mDate + '</span>' +
                  '<span style="text-align: right; font-weight: ' + (scoreHome > scoreAway ? 'bold' : 'normal') + '; color: ' + (scoreHome > scoreAway ? 'white' : 'var(--text-dim)') + ';">' + hTeam + '</span>' +
                  '<span style="font-weight: bold; color: var(--football);">' + scoreHome + ' - ' + scoreAway + '</span>' +
                  '<span style="text-align: left; font-weight: ' + (scoreAway > scoreHome ? 'bold' : 'normal') + '; color: ' + (scoreAway > scoreHome ? 'white' : 'var(--text-dim)') + ';">' + aTeam + '</span>' +
                '</div>';
            });
            detailsHtml += '</div>';
          }

          if (!detailsHtml) {
            detailsHtml = '<div class="empty-state" style="font-size: 0.75rem; padding: 10px;">Timeline, stats, and lineups are not available yet for this match.</div>';
          }

          detailsContainer.innerHTML = detailsHtml;
          detailsContainer.dataset.loaded = 'true';
        } catch (err) {
          detailsContainer.innerHTML = '<div class="empty-state" style="color: var(--accent); font-size: 0.75rem; padding: 10px;">⚠️ ' + err.message + '</div>';
        }
      } else {
        card.classList.remove('expanded');
      }
    });

    // Settings handling
    const settingsModal = document.getElementById('settings-modal');
    
    document.getElementById('open-settings').onclick = async () => {
      try {
        const res = await fetch('/api/settings');
        const settings = await res.json();
        if (settings.sync_interval_mins) {
          document.getElementById('sync-interval').value = settings.sync_interval_mins;
        }
        if (settings.detail_update_interval_secs !== undefined) {
          document.getElementById('detail-interval').value = settings.detail_update_interval_secs;
        }
      } catch (e) {}
      settingsModal.style.display = 'flex';
    };

    document.getElementById('close-settings').onclick = () => {
      settingsModal.style.display = 'none';
    };

    document.getElementById('save-settings').onclick = async () => {
      const interval = document.getElementById('sync-interval').value;
      const detailInterval = document.getElementById('detail-interval').value;
      const token = document.getElementById('api-token').value;

      const payload = { 
        sync_interval_mins: Number(interval),
        detail_update_interval_secs: Number(detailInterval)
      };
      if (token.trim() !== '') {
        payload.api_token = token;
      }

      try {
        const res = await fetch('/api/settings', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload)
        });
        if (res.ok) {
          alert('Settings saved successfully!');
          settingsModal.style.display = 'none';
        } else {
          alert('Failed to save settings.');
        }
      } catch (e) {
        alert('Error saving settings.');
      }
    };

    // Load initial data
    loadMatches();
    // Poll every 5 seconds for live updates
    setInterval(loadMatches, 5000);
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
