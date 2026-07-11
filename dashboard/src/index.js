let liveSchemaPromise = null;

async function ensureLiveSchema(env) {
  if (!liveSchemaPromise) {
    liveSchemaPromise = (async () => {
      const columns = await env.DB.prepare("PRAGMA table_info(matches)").all();
      const names = new Set((columns.results || []).map(column => String(column.name || "").toLowerCase()));
      if (!names.size) throw new Error("D1 schema is not initialized; run npm run d1:init");
      if (!names.has("is_live")) {
        try {
          await env.DB.prepare("ALTER TABLE matches ADD COLUMN is_live INTEGER NOT NULL DEFAULT 0").run();
        } catch (error) {
          if (!String(error && error.message).toLowerCase().includes("duplicate column")) throw error;
        }
        await env.DB.prepare(
          `UPDATE matches SET is_live = CASE
             WHEN sport_id = 1 AND status_id IN (2, 3, 4, 5, 6, 7) THEN 1
             WHEN sport_id = 2 AND status_id IN (2, 3, 4, 5, 6, 7, 9) THEN 1
             ELSE 0
           END`
        ).run();
      }
    })();
  }
  try {
    await liveSchemaPromise;
  } catch (error) {
    liveSchemaPromise = null;
    throw error;
  }
}

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

    const rawPayloadObject = (raw, fallback) => {
      let value = raw;
      if (typeof value === "string") {
        try {
          value = JSON.parse(value);
        } catch (_) {
          value = null;
        }
      }
      if (value && typeof value === "object" && !Array.isArray(value) && value.raw_payload) {
        let nested = value.raw_payload;
        if (typeof nested === "string") {
          try {
            nested = JSON.parse(nested);
          } catch (_) {
            nested = null;
          }
        }
        if (nested && typeof nested === "object" && !Array.isArray(nested)) {
          value = { ...value, ...nested };
          delete value.raw_payload;
        }
      }
      if (!value || typeof value !== "object") {
        value = fallback && typeof fallback === "object" ? { ...fallback } : {};
        delete value.raw_payload;
      }
      return sanitizeObj(value);
    };

    const serializeRawPayload = (raw, fallback) => JSON.stringify(rawPayloadObject(raw, fallback));
    const isLiveStatus = (sportId, statusId) => sportId === 1
      ? [2, 3, 4, 5, 6, 7].includes(Number(statusId))
      : sportId === 2 && [2, 3, 4, 5, 6, 7, 9].includes(Number(statusId));
    const terminalStatusPattern = /(^|\b)(ft|aet|full[\s-]*time|finished|ended|after penalties|cancel(?:led|ed)|postponed|abandoned|awarded|walkover)(\b|$)/i;
    const hasTerminalStatus = (payload) => {
      if (!payload || typeof payload !== "object") return false;
      const values = [
        payload.statusText, payload.status_text, payload.statusLabel, payload.status_label,
        payload.statusName, payload.stateName, payload.matchStatus, payload.state,
        payload.status && typeof payload.status === "object" ? payload.status.name || payload.status.label || payload.status.text : payload.status
      ];
      return values.some(value => typeof value === "string" && terminalStatusPattern.test(value.trim()));
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
        await ensureLiveSchema(env);
        const rawBody = await request.text();
        let data;
        try {
          data = JSON.parse(rawBody);
        } catch (e) {
          return new Response(JSON.stringify({ error: "Invalid JSON payload" }), { status: 400, headers: { "Content-Type": "application/json" } });
        }

        const {
          protocol_version, sync_id, dataset_id, dataset_created_at,
          generation_order, sport_id, captured_at, matches, match_details,
          teams, competitions
        } = data;
        const syncHeaders = { "Content-Type": "application/json", "Cache-Control": "no-store" };
        const rejectSync = (error) => new Response(JSON.stringify({ error }), { status: 400, headers: syncHeaders });

        if (protocol_version !== 2) return rejectSync("Unsupported protocol_version");
        if (typeof sync_id !== "string" || !sync_id.trim()) return rejectSync("Missing sync_id");
        if (typeof dataset_id !== "string" || !dataset_id.trim()) return rejectSync("Missing dataset_id");
        if (typeof dataset_created_at !== "string" || !Number.isFinite(Date.parse(dataset_created_at))) return rejectSync("Invalid dataset_created_at");
        if (!Number.isSafeInteger(generation_order) || generation_order < 1) return rejectSync("Invalid generation_order");
        if (![1, 2].includes(sport_id)) return rejectSync("sport_id must be 1 or 2");
        if (typeof captured_at !== "string" || !Number.isFinite(Date.parse(captured_at))) return rejectSync("Invalid captured_at");
        const collections = { competitions, teams, matches, match_details };
        for (const [name, rows] of Object.entries(collections)) {
          if (!Array.isArray(rows)) return rejectSync(`${name} must be an array`);
          for (const row of rows) {
            if (!row || row.dataset_id !== dataset_id) return rejectSync(`Missing or mixed dataset_id in ${name}`);
            if (row.sport_id !== sport_id) return rejectSync(`Mixed sport_id in ${name}`);
          }
        }

        const existingDataset = await env.DB.prepare(
          "SELECT created_at, generation_order FROM datasets WHERE dataset_id = ?1"
        ).bind(dataset_id).first();
        if (existingDataset && (existingDataset.created_at !== dataset_created_at || existingDataset.generation_order !== generation_order)) {
          return new Response(JSON.stringify({ error: "Dataset metadata is immutable" }), { status: 409, headers: syncHeaders });
        }
        const existingSport = await env.DB.prepare(
          "SELECT captured_at FROM dataset_sports WHERE dataset_id = ?1 AND sport_id = ?2"
        ).bind(dataset_id, sport_id).first();
        if (existingSport && Date.parse(captured_at) < Date.parse(existingSport.captured_at)) {
          return new Response(JSON.stringify({ error: "Stale sport batch" }), { status: 409, headers: syncHeaders });
        }

        const statements = [];

        statements.push(
          env.DB.prepare("INSERT OR IGNORE INTO datasets (dataset_id, created_at, generation_order, schema_generation) VALUES (?1, ?2, ?3, 2)").bind(dataset_id, dataset_created_at, generation_order)
        );

        const syncedIds = {
          competitions: [],
          teams: [],
          matches: [],
          match_details: []
        };

        // Save competitions
        if (competitions.length) {
          for (const c of competitions) {
            if (!c.id || !c.name) return rejectSync("Invalid competition row");
            const cleaned = sanitizeObj(c);
            statements.push(
              env.DB.prepare(
                "INSERT INTO competitions (id, sport_id, name, logo, slug, country_name, country_logo, raw_payload, synced, updated_at, dataset_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, datetime('now'), ?9) ON CONFLICT(id, dataset_id) DO UPDATE SET name=excluded.name, logo=excluded.logo, slug=excluded.slug, country_name=excluded.country_name, country_logo=excluded.country_logo, raw_payload=excluded.raw_payload, synced=1, updated_at=datetime('now')"
              ).bind(cleaned.id, cleaned.sport_id, cleaned.name, cleaned.logo || null, cleaned.slug || null, cleaned.country_name || null, cleaned.country_logo || null, serializeRawPayload(cleaned.raw_payload, cleaned), dataset_id)
            );
            syncedIds.competitions.push(c.id);
          }
        }

        // Save teams
        if (teams.length) {
          for (const t of teams) {
            if (!t.id || !t.name) return rejectSync("Invalid team row");
            const cleaned = sanitizeObj(t);
            statements.push(
              env.DB.prepare(
                "INSERT INTO teams (id, sport_id, name, logo, slug, raw_payload, synced, updated_at, dataset_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, datetime('now'), ?7) ON CONFLICT(id, dataset_id) DO UPDATE SET name=excluded.name, logo=excluded.logo, slug=excluded.slug, raw_payload=excluded.raw_payload, synced=1, updated_at=datetime('now')"
              ).bind(cleaned.id, cleaned.sport_id, cleaned.name, cleaned.logo || null, cleaned.slug || null, serializeRawPayload(cleaned.raw_payload, cleaned), dataset_id)
            );
            syncedIds.teams.push(t.id);
          }
        }

        // Save matches
        if (matches.length) {
          for (const m of matches) {
            if (!m.id || !m.competition_id || !m.home_team_id || !m.away_team_id) return rejectSync("Invalid match row");
            if (typeof m.is_live !== "boolean") return rejectSync("Invalid match is_live flag");
            const cleaned = sanitizeObj(m);
            const sourcePayload = rawPayloadObject(cleaned.raw_payload, cleaned);
            const effectiveIsLive = cleaned.is_live === true
              && isLiveStatus(cleaned.sport_id, cleaned.status_id)
              && !hasTerminalStatus(sourcePayload);
            statements.push(
              env.DB.prepare(
                "INSERT INTO matches (id, sport_id, competition_id, home_team_id, away_team_id, match_time, status_id, home_scores, away_scores, is_live, raw_payload, synced, updated_at, dataset_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, datetime('now'), ?12) ON CONFLICT(id, dataset_id) DO UPDATE SET sport_id=excluded.sport_id, competition_id=excluded.competition_id, home_team_id=excluded.home_team_id, away_team_id=excluded.away_team_id, match_time=excluded.match_time, status_id=excluded.status_id, home_scores=excluded.home_scores, away_scores=excluded.away_scores, is_live=excluded.is_live, raw_payload=excluded.raw_payload, synced=1, updated_at=datetime('now')"
              ).bind(cleaned.id, cleaned.sport_id, cleaned.competition_id, cleaned.home_team_id, cleaned.away_team_id, cleaned.match_time, cleaned.status_id, cleaned.home_scores, cleaned.away_scores, effectiveIsLive ? 1 : 0, JSON.stringify(sourcePayload), dataset_id)
            );
            syncedIds.matches.push(m.id);
          }
        }

        // Save match details
        if (match_details.length) {
          for (const d of match_details) {
            if (!d.match_id) return rejectSync("Invalid match detail row");
            const cleaned = sanitizeObj(d);
            statements.push(
              env.DB.prepare(
                "INSERT INTO match_details (match_id, sport_id, incidents, stats, lineups, odds, h2h, raw_payload, synced, last_updated, updated_at, dataset_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, datetime('now'), ?10) ON CONFLICT(match_id, dataset_id) DO UPDATE SET incidents=excluded.incidents, stats=excluded.stats, lineups=excluded.lineups, odds=excluded.odds, h2h=excluded.h2h, raw_payload=excluded.raw_payload, synced=1, last_updated=excluded.last_updated, updated_at=datetime('now')"
              ).bind(cleaned.match_id, cleaned.sport_id, cleaned.incidents, cleaned.stats, cleaned.lineups, cleaned.odds, cleaned.h2h, serializeRawPayload(cleaned.raw_payload, cleaned), cleaned.last_updated || null, dataset_id)
            );
            syncedIds.match_details.push(d.match_id);
          }
        }

        statements.push(
          env.DB.prepare(
            "INSERT INTO dataset_sports (dataset_id, sport_id, captured_at, synced_at) VALUES (?1, ?2, ?3, datetime('now')) ON CONFLICT(dataset_id, sport_id) DO UPDATE SET captured_at=excluded.captured_at, synced_at=datetime('now') WHERE excluded.captured_at >= dataset_sports.captured_at"
          ).bind(dataset_id, sport_id, captured_at)
        );
        statements.push(
          env.DB.prepare(
            `INSERT INTO settings (key, value) VALUES ('active_dataset_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value
             WHERE excluded.value = settings.value
                OR (SELECT generation_order FROM datasets WHERE dataset_id=excluded.value) > COALESCE((SELECT generation_order FROM datasets WHERE dataset_id=settings.value), -1)
                OR ((SELECT generation_order FROM datasets WHERE dataset_id=excluded.value) = COALESCE((SELECT generation_order FROM datasets WHERE dataset_id=settings.value), -1) AND excluded.value > settings.value)`
          ).bind(dataset_id)
        );
        await env.DB.batch(statements);

        const activeRow = await env.DB.prepare("SELECT value FROM settings WHERE key='active_dataset_id'").first();
        const isActive = activeRow && activeRow.value === dataset_id;

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
          sync_id,
          dataset_id: dataset_id,
          active: Boolean(isActive),
          sync_interval_mins: syncIntervalMins,
          synced_ids: syncedIds
        }), {
          headers: syncHeaders
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
        await ensureLiveSchema(env);
        const sportFilter = url.searchParams.get("sport_id");
        const parsedSportFilter = sportFilter === null ? null : Number(sportFilter);
        const group = url.searchParams.get("group") || "live";
        const limit = Math.min(Math.max(Number(url.searchParams.get("limit")) || 100, 1), 100);
        const offset = Math.max(Number(url.searchParams.get("offset")) || 0, 0);
        if (parsedSportFilter !== null && ![1, 2].includes(parsedSportFilter)) {
          return new Response(JSON.stringify({ error: "sport_id must be 1 or 2" }), {
            status: 400,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
          });
        }
        if (!["all", "live", "finished", "schedule"].includes(group)) {
          return new Response(JSON.stringify({ error: "group must be all, live, finished, or schedule" }), {
            status: 400,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
          });
        }

        let activeDatasetId = null;
        let datasetCreatedAt = null;
        let sportReadiness = { 1: { ready: false }, 2: { ready: false } };
        try {
          const settingRow = await env.DB.prepare("SELECT value FROM settings WHERE key = 'active_dataset_id'").first();
          if (settingRow && settingRow.value) {
            activeDatasetId = settingRow.value;
            const dsRow = await env.DB.prepare("SELECT created_at FROM datasets WHERE dataset_id = ?1").bind(activeDatasetId).first();
            if (dsRow) datasetCreatedAt = dsRow.created_at;
            const readyRows = await env.DB.prepare("SELECT sport_id, captured_at, synced_at FROM dataset_sports WHERE dataset_id = ?1").bind(activeDatasetId).all();
            for (const row of readyRows.results || []) {
              sportReadiness[row.sport_id] = { ready: true, captured_at: row.captured_at, synced_at: row.synced_at };
            }
          }
        } catch (_) {}

        if (!activeDatasetId) {
           return new Response(JSON.stringify({ active_dataset_id: null, sport_readiness: sportReadiness, matches: [] }), {
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
          });
        }

        let query = `
          SELECT
            m.id, m.sport_id, m.home_team_id, m.away_team_id, m.match_time, m.status_id, m.home_scores, m.away_scores, m.is_live, m.updated_at, m.raw_payload,
            md.updated_at as detail_updated_at,
            ht.name as home_name, ht.logo as home_logo, ht.slug as home_slug,
            at.name as away_name, at.logo as away_logo, at.slug as away_slug,
            c.name as comp_name, c.logo as comp_logo, c.country_name, c.country_logo
          FROM matches m
          LEFT JOIN teams ht ON m.home_team_id = ht.id AND m.dataset_id = ht.dataset_id
          LEFT JOIN teams at ON m.away_team_id = at.id AND m.dataset_id = at.dataset_id
          LEFT JOIN competitions c ON m.competition_id = c.id AND m.dataset_id = c.dataset_id
          LEFT JOIN match_details md ON m.id = md.match_id AND m.dataset_id = md.dataset_id
        `;
        const livePredicate = `(m.is_live = 1 AND ((m.sport_id = 1 AND m.status_id IN (2, 3, 4, 5, 6, 7)) OR (m.sport_id = 2 AND m.status_id IN (2, 3, 4, 5, 6, 7, 9))))`;
        const finishedPredicate = `((m.sport_id = 1 AND m.status_id IN (8, 9, 10, 11, 12, 13)) OR (m.sport_id = 2 AND m.status_id IN (8, 10, 11, 12, 13, 14)))`;
        const schedulePredicate = `(m.status_id = 1)`;
        const predicates = [`m.dataset_id = ?1`];
        const params = [activeDatasetId];
        if (parsedSportFilter !== null) {
          predicates.push("m.sport_id = ?2");
          params.push(parsedSportFilter);
        }
        if (group === "live") predicates.push(livePredicate);
        else if (group === "finished") predicates.push(finishedPredicate);
        else if (group === "schedule") predicates.push(schedulePredicate);
        else predicates.push(`(${livePredicate} OR ${finishedPredicate} OR ${schedulePredicate})`);
        const limitParam = params.length + 1;
        const offsetParam = params.length + 2;
        params.push(limit, offset);
        query += " WHERE " + predicates.join(" AND ") + ` ORDER BY CASE WHEN ${livePredicate} THEN 0 ELSE 1 END ASC, m.match_time DESC, m.id ASC LIMIT ?${limitParam} OFFSET ?${offsetParam}`;

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
            rawPayload = rawPayloadObject(row.raw_payload, {});
          } catch (_) {}

          const formattedRow = {
            ...row,
            home_scores: homeScores,
            away_scores: awayScores,
            raw_payload: rawPayload
          };
          if (group === "live" && (!formattedRow.is_live || !isLiveStatus(formattedRow.sport_id, formattedRow.status_id) || hasTerminalStatus(rawPayload))) return null;
          return formattedRow;
        }).filter(Boolean);

        return new Response(JSON.stringify({
          active_dataset_id: activeDatasetId,
          dataset_created_at: datasetCreatedAt,
          sport_readiness: sportReadiness,
          matches: formatted,
          pagination: { limit, offset, returned: formatted.length, has_more: formatted.length === limit }
        }), {
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
        });
      } catch (e) {
        return new Response(JSON.stringify({ error: e.message }), {
          status: 500,
          headers: { "Content-Type": "application/json", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
        });
      }
    }

    // 2b. API: Fetch match details (stats, lineups, incidents)
    if (url.pathname === "/api/matches/detail" && request.method === "GET") {
      try {
        const matchId = url.searchParams.get("match_id");
        if (!matchId) {
          return new Response(JSON.stringify({ error: "Missing match_id" }), { status: 400, headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" } });
        }
        let activeDatasetId = null;
        try {
          const settingRow = await env.DB.prepare("SELECT value FROM settings WHERE key = 'active_dataset_id'").first();
          if (settingRow) activeDatasetId = settingRow.value;
        } catch (_) {}

        if (!activeDatasetId) {
          return new Response(JSON.stringify({ error: "No active dataset" }), {
            status: 404,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
          });
        }

        const result = await env.DB.prepare(
          `SELECT d.match_id, d.sport_id, d.incidents, d.stats, d.lineups, d.odds, d.h2h, d.raw_payload, d.last_updated
           FROM match_details d
           JOIN matches m ON m.id = d.match_id AND m.dataset_id = d.dataset_id AND m.sport_id = d.sport_id
           WHERE d.match_id = ?1 AND d.dataset_id = ?2`
        ).bind(matchId, activeDatasetId).first();

        if (!result) {
          const matchExists = await env.DB.prepare(
            "SELECT 1 FROM matches WHERE id = ?1 AND dataset_id = ?2"
          ).bind(matchId, activeDatasetId).first();
          if (matchExists) {
            return new Response(JSON.stringify({
              pending: true,
              message: "Match details are still syncing for the active dataset"
            }), {
              status: 202,
              headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
            });
          }
          return new Response(JSON.stringify({ error: "Match not found in active dataset" }), {
            status: 404,
            headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
          });
        }

        const safeParse = (str, fallback) => {
          try {
            return JSON.parse(str || fallback);
          } catch (_) {
            return JSON.parse(fallback);
          }
        };

        const rawPayload = rawPayloadObject(result.raw_payload, {});
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
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
        });
      } catch (e) {
        return new Response(JSON.stringify({ error: e.message }), {
          status: 500,
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*", "Cache-Control": "no-store, no-cache, must-revalidate", "Pragma": "no-cache", "Expires": "0" }
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
  <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&display=swap" rel="stylesheet">
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

    .match-group-title {
      position: sticky;
      top: 0;
      z-index: 2;
      padding: 8px 10px;
      border-radius: 8px;
      background: var(--panel);
      color: var(--text-dim);
      font-size: 0.78rem;
      font-weight: 800;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }

    .match-group-title.live { color: var(--success); }

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

    /* AiScore-inspired live dashboard shell */
    :root {
      --bg: #f2f4f7;
      --panel: #ffffff;
      --card: #ffffff;
      --text: #172033;
      --text-dim: #8791a3;
      --primary: #1769e0;
      --primary-light: #2f7dec;
      --accent: #f04438;
      --success: #13a36f;
      --football: #1769e0;
      --basketball: #f59e0b;
      --line: #e7ebf0;
    }

    body {
      font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
      background: var(--bg);
      color: var(--text);
    }

    .topbar {
      position: sticky;
      top: 0;
      z-index: 30;
      padding: 0;
      color: #fff;
      background: linear-gradient(105deg, #1262d5 0%, #1976ea 100%);
      border: 0;
      box-shadow: 0 2px 10px rgba(21, 80, 160, 0.2);
    }

    .topbar-inner,
    .sports-nav-inner {
      width: min(1120px, 100%);
      margin: 0 auto;
      display: flex;
      align-items: center;
    }

    .topbar-inner {
      min-height: 64px;
      padding: 0 20px;
      justify-content: space-between;
      gap: 18px;
    }

    .brand {
      display: flex;
      align-items: center;
      gap: 10px;
      min-width: 0;
    }

    .brand-mark {
      width: 34px;
      height: 34px;
      display: grid;
      place-items: center;
      flex: 0 0 auto;
      border-radius: 10px;
      background: #fff;
      color: var(--primary);
      font-weight: 900;
      font-size: 1.15rem;
      box-shadow: 0 4px 12px rgba(0, 35, 95, 0.2);
    }

    .brand-name {
      font-size: 1.28rem;
      line-height: 1;
      font-weight: 800;
      letter-spacing: -0.03em;
    }

    .brand-subtitle {
      display: block;
      margin-top: 3px;
      color: rgba(255,255,255,.72);
      font-size: .68rem;
      font-weight: 600;
      letter-spacing: .05em;
      text-transform: uppercase;
    }

    .topbar-actions {
      display: flex;
      align-items: center;
      gap: 10px;
      min-width: 0;
    }

    #sync-info {
      max-width: 460px;
      color: rgba(255,255,255,.82);
      font-size: .72rem;
      line-height: 1.35;
      text-align: right;
    }

    #sync-info.refresh-error,
    #sync-info.refresh-stale { color: #fff3bd !important; }

    .settings-btn {
      width: 38px;
      height: 38px;
      display: grid;
      place-items: center;
      flex: 0 0 auto;
      border: 1px solid rgba(255,255,255,.28);
      border-radius: 50%;
      color: #fff;
      background: rgba(255,255,255,.12);
      font-size: 1rem;
    }

    .settings-btn:hover,
    .settings-btn:focus-visible { color: #fff; background: rgba(255,255,255,.22); outline: 2px solid rgba(255,255,255,.7); }

    .sports-nav {
      position: sticky;
      top: 64px;
      z-index: 25;
      background: #fff;
      border-bottom: 1px solid var(--line);
      box-shadow: 0 2px 6px rgba(27, 42, 70, .04);
    }

    .sports-nav-inner {
      min-height: 58px;
      padding: 0 14px;
      gap: 4px;
      overflow-x: auto;
      scrollbar-width: none;
    }

    .sports-nav-inner::-webkit-scrollbar { display: none; }

    .sport-tab {
      position: relative;
      min-width: 104px;
      min-height: 58px;
      padding: 7px 14px;
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
      border: 0;
      color: #687386;
      background: transparent;
      font: inherit;
      font-size: .82rem;
      font-weight: 600;
      cursor: pointer;
      white-space: nowrap;
    }

    .sport-tab::after {
      content: '';
      position: absolute;
      left: 18px;
      right: 18px;
      bottom: 0;
      height: 3px;
      border-radius: 3px 3px 0 0;
      background: transparent;
    }

    .sport-tab.active { color: var(--primary); }
    .sport-tab.active::after { background: var(--primary); }
    .sport-tab-icon { width: 18px; height: 18px; display: inline-flex; align-items: center; justify-content: center; flex: 0 0 auto; }
    .sport-tab-icon svg { width: 100%; height: 100%; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
    .settings-btn svg { width: 17px; height: 17px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }

    main.dashboard-shell {
      width: min(1120px, 100%);
      max-width: none;
      margin: 0 auto;
      padding: 22px 20px 46px;
      display: grid;
      grid-template-columns: minmax(0, 1fr) 272px;
      align-items: start;
      gap: 18px;
    }

    .feed-panel,
    .summary-card {
      border: 1px solid var(--line);
      border-radius: 10px;
      background: #fff;
      box-shadow: 0 4px 14px rgba(32, 50, 84, .045);
    }

    .feed-toolbar {
      min-height: 58px;
      padding: 0 16px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      border-bottom: 1px solid var(--line);
    }

    .filter-tabs { display: flex; align-items: stretch; align-self: stretch; overflow-x: auto; }
    .filter-tabs .feed-count { align-self: center; margin-left: 8px; }

    .filter-tab {
      position: relative;
      min-width: 70px;
      padding: 0 13px;
      border: 0;
      color: #7c8697;
      background: transparent;
      font: inherit;
      font-size: .8rem;
      font-weight: 600;
      cursor: pointer;
    }

    .filter-tab::after {
      content: '';
      position: absolute;
      left: 12px;
      right: 12px;
      bottom: 0;
      height: 2px;
      background: transparent;
    }

    .filter-tab.active { color: var(--primary); }
    .filter-tab.active::after { background: var(--primary); }

    .feed-count {
      flex: 0 0 auto;
      padding: 5px 9px;
      border-radius: 999px;
      color: var(--primary);
      background: #edf5ff;
      font-size: .72rem;
      font-weight: 700;
    }

    .date-strip {
      min-height: 48px;
      padding: 0 16px;
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 18px;
      color: #556176;
      background: #fbfcfd;
      border-bottom: 1px solid var(--line);
      font-size: .8rem;
      font-weight: 700;
    }

    .date-arrow { color: #b1b8c4; font-size: 1.05rem; }
    .live-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--accent); box-shadow: 0 0 0 4px rgba(240,68,56,.1); }

    .matches-feed { min-height: 360px; }

    .competition-group + .competition-group { border-top: 8px solid var(--bg); }

    .competition-header {
      min-height: 45px;
      padding: 8px 14px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      background: #fafbfc;
      border-bottom: 1px solid var(--line);
    }

    .competition-info { display: flex; align-items: center; gap: 9px; min-width: 0; }
    .competition-logo { width: 23px; height: 23px; border-radius: 50%; object-fit: contain; background: #fff; }
    .competition-name { overflow: hidden; color: #384459; font-size: .76rem; font-weight: 700; text-overflow: ellipsis; white-space: nowrap; }
    .competition-country { display: block; margin-top: 2px; color: #9aa3b1; font-size: .65rem; font-weight: 500; }
    .competition-live-count { color: var(--accent); font-size: .68rem; font-weight: 700; white-space: nowrap; }

    .match-card {
      padding: 0;
      gap: 0;
      border: 0;
      border-bottom: 1px solid var(--line);
      border-radius: 0;
      background: #fff;
      box-shadow: none;
      transition: background .18s ease;
    }

    .match-card:last-child { border-bottom: 0; }
    .match-card:hover,
    .match-card:focus-visible { border-color: var(--line); background: #f8fbff; box-shadow: none; transform: none; outline: 2px solid rgba(23,105,224,.18); outline-offset: -2px; }

    .match-row {
      min-height: 92px;
      padding: 13px 14px;
      display: grid;
      grid-template-columns: 72px minmax(0, 1fr) 44px 18px;
      align-items: center;
      gap: 11px;
    }

    .match-state { align-self: stretch; display: flex; flex-direction: column; align-items: flex-start; justify-content: center; border-right: 1px solid #edf0f4; }
    .state-label { color: var(--accent); font-size: .72rem; font-weight: 800; }
    .state-time { margin-top: 4px; color: #98a1af; font-size: .66rem; }

    .teams-stack { min-width: 0; display: flex; flex-direction: column; gap: 10px; }
    .team-line { min-width: 0; display: flex; align-items: center; gap: 9px; }
    .team-logo { width: 24px; height: 24px; flex: 0 0 auto; object-fit: contain; }
    .team-name { overflow: hidden; color: #263146; font-size: .78rem; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }

    .scores-stack { display: flex; flex-direction: column; align-items: center; gap: 12px; color: #172033; font-size: .9rem; font-weight: 800; }
    .score-value.live { color: var(--primary); }
    .expand-chevron { color: #b0b7c3; font-size: .82rem; transform: rotate(0); transition: transform .2s ease; }
    .match-card.expanded .expand-chevron { transform: rotate(90deg); }

    .match-meta {
      padding: 0 14px 9px 97px;
      color: #9aa3b2;
      font-size: .64rem;
      line-height: 1.35;
    }

    .red-card-badge,
    .yellow-card-badge { font-size: .58rem; }

    .match-details-container {
      margin: 0;
      padding: 0 16px 16px 97px;
      border: 0;
      background: #fbfcfe;
    }

    .detail-section-title { color: var(--primary); }
    .detail-grid dt { color: #8791a3; }
    .detail-grid dd { color: #334057; }
    .stat-bar-container { background: #edf0f4; }
    .stat-bar-home { background: var(--primary); }

    .empty-state {
      min-height: 280px;
      padding: 62px 20px;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 8px;
      color: #8c96a7;
      font-size: .8rem;
    }

    .empty-icon { width: 48px; height: 48px; display: grid; place-items: center; border-radius: 50%; color: var(--primary); background: #edf5ff; font-size: 1.25rem; }

    .summary-panel { display: flex; flex-direction: column; gap: 14px; }
    .summary-card { padding: 16px; }
    .summary-heading { margin-bottom: 14px; color: #334057; font-size: .78rem; font-weight: 800; }
    .summary-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 9px; }
    .summary-stat { padding: 12px 10px; border: 1px solid #edf0f4; border-radius: 8px; background: #fbfcfd; }
    .summary-value { display: block; color: #243148; font-size: 1.15rem; font-weight: 800; }
    .summary-label { display: block; margin-top: 4px; color: #98a1af; font-size: .64rem; }
    .source-line { display: flex; align-items: flex-start; gap: 9px; color: #6e7889; font-size: .7rem; line-height: 1.5; }
    .source-line + .source-line { margin-top: 10px; }
    .source-icon { width: 22px; height: 22px; display: grid; place-items: center; flex: 0 0 auto; border-radius: 50%; color: var(--primary); background: #edf5ff; font-size: .65rem; }

    .modal { background: rgba(20, 31, 50, .55); }
    .modal-content { color: var(--text); background: #fff; border-color: var(--line); }
    .form-group input, .form-group select { color: var(--text); background: #f7f9fb; border-color: #dce2e9; }
    .btn-cancel { color: #5f6b7e; background: #edf0f4; }
    .btn-save { background: var(--primary); }

    @media (max-width: 860px) {
      main.dashboard-shell { grid-template-columns: 1fr; padding: 14px 12px 34px; }
      .summary-panel { order: -1; display: grid; grid-template-columns: 1fr 1fr; }
    }

    @media (max-width: 620px) {
      .topbar-inner { min-height: 58px; padding: 0 14px; }
      .sports-nav { top: 58px; }
      #sync-info { display: none; }
      .brand-subtitle { display: none; }
      .sport-tab { min-width: 92px; }
      main.dashboard-shell { padding: 10px 0 28px; }
      .feed-panel { border-left: 0; border-right: 0; border-radius: 0; }
      .summary-panel { margin: 0 10px; grid-template-columns: 1fr; }
      .summary-panel .summary-card:last-child { display: none; }
      .feed-toolbar { padding: 0 7px; }
      .filter-tab { min-width: 63px; padding: 0 8px; }
      .match-row { grid-template-columns: 62px minmax(0, 1fr) 36px 14px; padding: 12px 10px; gap: 8px; }
      .match-meta { padding-left: 80px; padding-right: 10px; }
      .match-details-container { padding-left: 16px; padding-right: 16px; }
      .competition-header { padding-left: 10px; padding-right: 10px; }
    }
  </style>
</head>
<body>

  <header class="topbar">
    <div class="topbar-inner">
      <div class="brand">
        <span class="brand-mark">E</span>
        <span><span class="brand-name">EarnScore</span><span class="brand-subtitle">Live score center</span></span>
      </div>
      <div class="topbar-actions">
        <div id="sync-info" role="status" aria-live="polite">Waiting for the latest live capture...</div>
        <button class="settings-btn" id="open-settings" aria-label="System Settings"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-1.6v-.2h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1z"></path></svg></button>
      </div>
    </div>
  </header>

  <nav class="sports-nav" aria-label="Sports">
    <div class="sports-nav-inner">
      <button class="sport-tab active" type="button" data-sport="0"><span class="sport-tab-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="3" width="7" height="7" rx="1"></rect><rect x="14" y="3" width="7" height="7" rx="1"></rect><rect x="3" y="14" width="7" height="7" rx="1"></rect><rect x="14" y="14" width="7" height="7" rx="1"></rect></svg></span>All Sports</button>
      <button class="sport-tab" type="button" data-sport="1"><span class="sport-tab-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"></circle><path d="m9 10 3-2 3 2-1 3h-4zM12 8V3M10 13l-4 3m8-3 4 3M7 7l2 3m8-3-2 3M8 20l-2-4m10 4 2-4"></path></svg></span>Football</button>
      <button class="sport-tab" type="button" data-sport="2"><span class="sport-tab-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"></circle><path d="M3.5 9h17M3.5 15h17M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"></path></svg></span>Basketball</button>
    </div>
  </nav>

  <main class="dashboard-shell">
    <section class="feed-panel" aria-labelledby="feed-title">
      <div class="feed-toolbar">
        <div class="filter-tabs" role="tablist" aria-label="Match status">
          <button class="filter-tab active" type="button" data-group="live" role="tab" aria-selected="true">Live</button>
          <span class="feed-count">In-play matches only</span>
        </div>
        <span class="feed-count" id="result-count">0 matches</span>
      </div>
      <div class="date-strip" id="feed-title"><span class="date-arrow">‹</span><span class="live-dot"></span><span>Today · Live Center</span><span class="date-arrow">›</span></div>
      <div class="matches-feed" id="matches-feed"><div class="empty-state"><span class="empty-icon">↻</span>Loading every live match...</div></div>
    </section>

    <aside class="summary-panel" aria-label="Live dashboard summary">
      <section class="summary-card">
        <h2 class="summary-heading">Live Dashboard</h2>
        <div class="summary-grid">
          <div class="summary-stat"><strong class="summary-value" id="live-total">0</strong><span class="summary-label">Live now</span></div>
          <div class="summary-stat"><strong class="summary-value" id="football-count">0</strong><span class="summary-label">Football</span></div>
          <div class="summary-stat"><strong class="summary-value" id="basketball-count">0</strong><span class="summary-label">Basketball</span></div>
          <div class="summary-stat"><strong class="summary-value" id="league-count">0</strong><span class="summary-label">Competitions</span></div>
        </div>
      </section>
      <section class="summary-card">
        <h2 class="summary-heading">Data Source</h2>
        <div class="source-line"><span class="source-icon">✓</span><span id="source-state">Waiting for the active dataset</span></div>
        <div class="source-line"><span class="source-icon">↻</span><span id="source-time">No capture received yet</span></div>
      </section>
    </aside>
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

    function isTerminalStatusLabel(value) {
      if (typeof value !== 'string') return false;
      return /(^|\b)(ft|aet|full[\s-]*time|finished|ended|after penalties|cancel(?:led|ed)|postponed|abandoned|awarded|walkover)(\b|$)/i.test(value.trim());
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
        is_live: raw.is_live === true || raw.is_live === 1 || raw.is_live === '1',
        status_label: rawStatusLabel(rawPayload),
        home_scores: normalizeScores(raw.home_scores),
        away_scores: normalizeScores(raw.away_scores),
        home_name: safeText(raw.home_name, 'Team A'),
        away_name: safeText(raw.away_name, 'Team B'),
        comp_name: safeText(raw.comp_name, 'Other League'),
        country_name: safeText(raw.country_name, ''),
        country_logo: typeof raw.country_logo === 'string' ? raw.country_logo : '',
        home_logo: typeof raw.home_logo === 'string' ? raw.home_logo : '',
        away_logo: typeof raw.away_logo === 'string' ? raw.away_logo : '',
        comp_logo: typeof raw.comp_logo === 'string' ? raw.comp_logo : '',
        raw_payload: rawPayload,
        version: safeText(raw.updated_at, '') + '|' + safeText(raw.detail_updated_at, '') + '|' + String(raw.is_live)
      };
    }

    function normalizeMatchesPayload(payload) {
      if (!Array.isArray(payload)) throw new Error('Live matches response must be an array');
      return payload
        .map(normalizeMatch)
        .filter(match => match && match.is_live
          && getStatusInfo(match.sport_id, match.status_id, match.status_label).state === 'live'
          && !isTerminalStatusLabel(match.status_label))
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
          7: { label: 'Penalty Shootout', state: 'live' },
          8: { label: 'FT', state: 'finished' },
          9: { label: 'Postponed', state: 'finished' },
          10: { label: 'Cancelled', state: 'finished' },
          11: { label: 'Abandoned', state: 'finished' },
          12: { label: 'Cancelled', state: 'finished' },
          13: { label: 'Ended', state: 'finished' }
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
          8: { label: 'Finished', state: 'finished' },
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
      const statusText = clock || statusInfo.label;
      const homeTotal = scoreAt(match.home_scores, 0);
      const awayTotal = scoreAt(match.away_scores, 0);
      const meta = [];
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
        if (homeHalf !== null || awayHalf !== null) meta.push('HT ' + displayScore(homeHalf) + ' - ' + displayScore(awayHalf));
        const homeCorners = scoreAt(match.home_scores, 4);
        const awayCorners = scoreAt(match.away_scores, 4);
        if (homeCorners !== null || awayCorners !== null) meta.push('Corners ' + displayScore(homeCorners) + ' - ' + displayScore(awayCorners));
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
        if (periods.length) meta.push(periods.join(' · '));
      }

      const isExpanded = preserved.expandedId === match.id;
      const sameVersion = isExpanded && preserved.version === match.version;
      const detailsState = sameVersion ? preserved.detailsState : 'false';
      const detailsHtml = sameVersion ? preserved.detailsHtml : '';
      const homeLogo = resolveAssetUrl(match.home_logo, match.sport_id, 'team');
      const awayLogo = resolveAssetUrl(match.away_logo, match.sport_id, 'team');
      const liveScoreClass = statusInfo.state === 'live' ? ' live' : '';
      const accessibleLabel = match.home_name + ' versus ' + match.away_name + ', ' + displayScore(homeTotal) + ' to ' + displayScore(awayTotal);

      return '<article class="match-card ' + (isExpanded ? 'expanded' : '') + '"' +
        ' data-match-id="' + escapeHtml(match.id) + '" data-home-id="' + escapeHtml(match.home_team_id) + '"' +
        ' data-away-id="' + escapeHtml(match.away_team_id) + '" data-version="' + escapeHtml(match.version) + '"' +
        ' tabindex="0" role="button" aria-label="' + escapeHtml(accessibleLabel) + '" aria-expanded="' + (isExpanded ? 'true' : 'false') + '">' +
          '<div class="match-row">' +
            '<div class="match-state"><span class="state-label">' + escapeHtml(statusText) + '</span><span class="state-time">' + escapeHtml(formatMatchTime(match.match_time)) + '</span></div>' +
            '<div class="teams-stack">' +
              '<div class="team-line">' + renderImage(homeLogo, 'team-logo', DEFAULT_LOGOS[match.sport_id]) + '<span class="team-name">' + escapeHtml(match.home_name) + ' ' + homeBadges + '</span></div>' +
              '<div class="team-line">' + renderImage(awayLogo, 'team-logo', DEFAULT_LOGOS[match.sport_id]) + '<span class="team-name">' + escapeHtml(match.away_name) + ' ' + awayBadges + '</span></div>' +
            '</div>' +
            '<div class="scores-stack"><span class="score-value' + liveScoreClass + '">' + displayScore(homeTotal) + '</span><span class="score-value' + liveScoreClass + '">' + displayScore(awayTotal) + '</span></div>' +
            '<span class="expand-chevron" aria-hidden="true">›</span>' +
          '</div>' +
          (meta.length ? '<div class="match-meta">' + escapeHtml(meta.join(' · ')) + '</div>' : '') +
          '<div class="match-details-container" data-loaded="' + escapeHtml(detailsState) + '">' + detailsHtml + '</div></article>';
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
        windowScroll: window.scrollY
      };
    }

    let currentDatasetId = null;
    let loadedMatches = [];
    let lastPayload = null;
    let activeSportId = 0;
    let activeGroup = 'live';

    function renderCompetitionGroups(matches, preserved) {
      const groups = new Map();
      matches.forEach(match => {
        const key = [match.sport_id, match.comp_name, match.country_name].join('|');
        if (!groups.has(key)) groups.set(key, []);
        groups.get(key).push(match);
      });

      return Array.from(groups.values())
        .sort((a, b) => {
          const aTime = Math.min(...a.map(match => match.match_time));
          const bTime = Math.min(...b.map(match => match.match_time));
          return aTime - bTime || a[0].comp_name.localeCompare(b[0].comp_name);
        })
        .map(group => {
          const sample = group[0];
          const liveCount = group.filter(match => getStatusInfo(match.sport_id, match.status_id, match.status_label).state === 'live').length;
          const competitionLogo = resolveAssetUrl(sample.comp_logo, sample.sport_id, 'competition');
          const country = sample.country_name || (sample.sport_id === 1 ? 'Football' : 'Basketball');
          const countLabel = liveCount ? liveCount + ' live' : group.length + (group.length === 1 ? ' match' : ' matches');
          return '<section class="competition-group">' +
            '<header class="competition-header"><div class="competition-info">' +
              renderImage(competitionLogo, 'competition-logo', '') +
              '<span class="competition-name">' + escapeHtml(sample.comp_name) + '<span class="competition-country">' + escapeHtml(country) + '</span></span>' +
            '</div><span class="competition-live-count">' + escapeHtml(countLabel) + '</span></header>' +
            group.map(match => renderMatchCard(match, preserved)).join('') +
          '</section>';
        }).join('');
    }

    function emptyFeedMessage(payload) {
      if (!payload.active_dataset_id) return 'Waiting for the first live dataset.';
      const readiness = payload.sport_readiness || {};
      const relevantSports = activeSportId ? [activeSportId] : [1, 2];
      if (relevantSports.every(sportId => !(readiness[sportId] && readiness[sportId].ready))) {
        return activeSportId ? 'This sport has not synced in the active generation yet.' : 'Sports are still syncing for the active generation.';
      }
      if (activeGroup === 'live') return 'There are no matches in play right now.';
      if (activeGroup === 'finished') return 'No finished matches are available in the latest capture.';
      if (activeGroup === 'schedule') return 'No scheduled matches are available in the latest capture.';
      return 'No matches are available in the latest capture.';
    }

    function parseSourceTime(value) {
      if (typeof value !== 'string' || !value) return NaN;
      const normalized = value.includes('T') || /[zZ]|[+-]\d\d:\d\d$/.test(value) ? value : value.replace(' ', 'T') + 'Z';
      return Date.parse(normalized);
    }

    function renderMatches(payload) {
      const activeDatasetId = payload.active_dataset_id || null;
      const generationChanged = activeDatasetId !== currentDatasetId;
      const preserved = generationChanged ? {
        expandedId: '', version: '', detailsHtml: '', detailsState: 'false', focusedId: '', windowScroll: window.scrollY
      } : captureRenderState();

      if (generationChanged) document.getElementById('matches-feed').innerHTML = '';
      currentDatasetId = activeDatasetId;
      loadedMatches = Array.isArray(payload.matches) ? payload.matches : [];
      lastPayload = payload;

      const visibleMatches = loadedMatches.filter(match => !activeSportId || match.sport_id === activeSportId);
      const football = loadedMatches.filter(match => match.sport_id === 1);
      const basketball = loadedMatches.filter(match => match.sport_id === 2);
      const liveMatches = loadedMatches.filter(match => match.is_live && getStatusInfo(match.sport_id, match.status_id, match.status_label).state === 'live');
      const leagueCount = new Set(visibleMatches.map(match => match.sport_id + '|' + match.comp_name + '|' + match.country_name)).size;
      const feed = document.getElementById('matches-feed');

      feed.innerHTML = visibleMatches.length
        ? renderCompetitionGroups(visibleMatches, preserved)
        : '<div class="empty-state"><span class="empty-icon">' + (activeGroup === 'live' ? '○' : '–') + '</span>' + escapeHtml(emptyFeedMessage(payload)) + '</div>';

      document.getElementById('result-count').textContent = visibleMatches.length + (visibleMatches.length === 1 ? ' match' : ' matches');
      document.getElementById('live-total').textContent = liveMatches.length;
      document.getElementById('football-count').textContent = football.length;
      document.getElementById('basketball-count').textContent = basketball.length;
      document.getElementById('league-count').textContent = leagueCount;

      if (preserved.focusedId) {
        const focused = Array.from(document.querySelectorAll('.match-card')).find(card => card.dataset.matchId === preserved.focusedId);
        if (focused) focused.focus({ preventScroll: true });
      }
      requestAnimationFrame(() => window.scrollTo({ top: preserved.windowScroll, behavior: 'auto' }));
      const expanded = document.querySelector('.match-card.expanded');
      if (expanded && expanded.querySelector('.match-details-container').dataset.loaded !== 'true') toggleCard(expanded, true);

      const readiness = payload.sport_readiness || {};
      if (activeDatasetId) {
        const shortDatasetId = activeDatasetId.split('-')[0] || activeDatasetId.substring(0, 8);
        const readyValues = Object.values(readiness).filter(item => item && item.ready);
        const latestCapture = readyValues.map(item => parseSourceTime(item.captured_at)).filter(Number.isFinite).sort((a, b) => b - a)[0];
        const latestSync = readyValues.map(item => parseSourceTime(item.synced_at)).filter(Number.isFinite).sort((a, b) => b - a)[0];
        const stale = Number.isFinite(latestCapture) && Date.now() - latestCapture > 120000;
        let statusText = 'Dataset ' + shortDatasetId;
        if (Number.isFinite(latestCapture)) statusText += ' · Source ' + new Date(latestCapture).toLocaleTimeString();
        if (Number.isFinite(latestSync)) statusText += ' · Synced ' + new Date(latestSync).toLocaleTimeString();
        if (stale) statusText += ' · Source is stale';
        setRefreshStatus(statusText, stale ? 'stale' : '');
        document.getElementById('source-state').textContent = 'Active dataset ' + shortDatasetId + (stale ? ' · source is stale' : ' · live generation');
        document.getElementById('source-time').textContent = Number.isFinite(latestCapture)
          ? 'Last source capture ' + new Date(latestCapture).toLocaleString()
          : 'Waiting for a sport capture';
      } else {
        setRefreshStatus('Waiting for first sync...', 'stale');
        document.getElementById('source-state').textContent = 'Waiting for the active dataset';
        document.getElementById('source-time').textContent = 'No capture received yet';
      }
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
    let requestSequence = 0;

    function scheduleNextPoll(delay) {
      clearTimeout(pollTimer);
      pollTimer = null;
      if (!document.hidden) pollTimer = setTimeout(loadMatches, delay);
    }

    async function fetchEveryMatchPage(group, signal) {
      const pageSize = 100;
      let offset = 0;
      let firstPayload = null;
      let expectedDatasetId;
      const byId = new Map();

      for (let page = 0; page < 100; page += 1) {
        const response = await fetch('/api/matches/live?group=' + encodeURIComponent(group) + '&limit=' + pageSize + '&offset=' + offset, { signal, cache: 'no-store' });
        if (!response.ok) throw new Error('Live matches request failed (' + response.status + ')');
        const payload = await response.json();
        if (!isRecord(payload) || !Array.isArray(payload.matches)) throw new Error('Malformed live matches response');
        if (!firstPayload) {
          firstPayload = payload;
          expectedDatasetId = payload.active_dataset_id || null;
        } else if ((payload.active_dataset_id || null) !== expectedDatasetId) {
          throw new Error('Dataset changed while loading the live feed');
        }

        payload.matches.forEach(match => {
          if (match && match.id !== undefined) byId.set(String(match.id), match);
        });
        const returned = Number(payload.pagination && payload.pagination.returned);
        const pageReturned = Number.isFinite(returned) ? returned : payload.matches.length;
        const hasMore = Boolean(payload.pagination && payload.pagination.has_more);
        if (!hasMore || pageReturned === 0 || pageReturned < pageSize) break;
        offset += pageReturned;
        if (offset > 10000) throw new Error('Live feed exceeded the safe page limit');
      }

      const result = firstPayload || { active_dataset_id: null, sport_readiness: {}, matches: [] };
      result.matches = normalizeMatchesPayload(Array.from(byId.values()));
      result.pagination = { limit: pageSize, offset: 0, returned: result.matches.length, has_more: false };
      return result;
    }

    async function loadMatches() {
      const sequence = ++requestSequence;
      if (activeAbortController) activeAbortController.abort();
      matchesInFlight = true;
      const controller = new AbortController();
      activeAbortController = controller;
      let succeeded = false;
      try {
        const payload = await fetchEveryMatchPage(activeGroup, controller.signal);
        if (sequence !== requestSequence) return;
        renderMatches(payload);
        lastSuccessfulRefresh = new Date();
        consecutiveFailures = 0;
        succeeded = true;
      } catch (err) {
        if (err.name !== 'AbortError' && sequence === requestSequence) {
          consecutiveFailures += 1;
          const retryMs = Math.min(BASE_POLL_MS * Math.pow(2, consecutiveFailures), MAX_POLL_MS);
          if (lastSuccessfulRefresh) {
            setRefreshStatus('Refresh failed; showing data from ' + lastSuccessfulRefresh.toLocaleTimeString() + '. Retrying in ' + Math.round(retryMs / 1000) + 's.', 'stale');
          } else {
            setRefreshStatus('Unable to load matches. Retrying in ' + Math.round(retryMs / 1000) + 's.', 'error');
            document.getElementById('matches-feed').innerHTML = '<div class="empty-state"><span class="empty-icon">!</span>Unable to load matches. Automatic retry is active.</div>';
          }
          console.error('Error fetching matches:', err);
        }
      } finally {
        if (sequence === requestSequence) {
          matchesInFlight = false;
          activeAbortController = null;
          scheduleNextPoll(succeeded ? BASE_POLL_MS : Math.min(BASE_POLL_MS * Math.pow(2, consecutiveFailures), MAX_POLL_MS));
        }
      }
    }

    document.querySelectorAll('.sport-tab').forEach(button => {
      button.addEventListener('click', () => {
        activeSportId = Number(button.dataset.sport) || 0;
        document.querySelectorAll('.sport-tab').forEach(item => item.classList.toggle('active', item === button));
        if (lastPayload) renderMatches(lastPayload);
      });
    });

    document.querySelectorAll('.filter-tab').forEach(button => {
      button.addEventListener('click', () => {
        if (button.dataset.group === activeGroup) return;
        activeGroup = button.dataset.group;
        document.querySelectorAll('.filter-tab').forEach(item => {
          const selected = item === button;
          item.classList.toggle('active', selected);
          item.setAttribute('aria-selected', selected ? 'true' : 'false');
        });
        document.getElementById('matches-feed').innerHTML = '<div class="empty-state"><span class="empty-icon">↻</span>Loading every ' + escapeHtml(activeGroup) + ' match...</div>';
        loadMatches();
      });
    });

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
        const response = await fetch('/api/matches/detail?match_id=' + encodeURIComponent(card.dataset.matchId), { cache: 'no-store' });
        if (response.status === 202) {
          detailsContainer.innerHTML = '<div class="empty-state" style="font-size: 0.8rem; padding: 12px;">Details are still syncing. Please open this match again shortly.</div>';
          detailsContainer.dataset.loaded = 'pending';
          return;
        }
        if (response.status === 404 || response.status === 409) {
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
