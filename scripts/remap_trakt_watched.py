#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import json
import shutil
import sqlite3
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path


USER_AGENT = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
    "AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/127.0.0.0 Safari/537.36"
)


@dataclass
class SeriesRow:
    id: str
    name: str
    imdb: str | None
    tmdb: int | None
    trakt: int | None
    tvdb: int | None


@dataclass
class EpisodeRow:
    serie_ref: str
    season: int
    number: int
    imdb: str | None
    tmdb: int | None
    trakt: int | None
    tvdb: int | None

    @property
    def redseat_id(self) -> str:
        return f"redseat:{self.serie_ref}x{self.season}x{self.number}"


class TraktClient:
    def __init__(self, client_id: str) -> None:
        self.client_id = client_id

    def _headers(self) -> dict[str, str]:
        return {
            "trakt-api-version": "2",
            "trakt-api-key": self.client_id,
            "Accept": "application/json",
            "Content-Type": "application/json",
            "User-Agent": USER_AGENT,
        }

    def get_json(self, path: str, retries: int = 4) -> object:
        url = f"https://api.trakt.tv{path}"
        last_error: Exception | None = None
        for attempt in range(retries):
            request = urllib.request.Request(url, headers=self._headers())
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    payload = response.read().decode("utf-8")
                    return json.loads(payload)
            except urllib.error.HTTPError as exc:
                body = exc.read().decode("utf-8", errors="replace")
                last_error = RuntimeError(f"{exc.code} for {url}: {body[:400]}")
                if exc.code == 429:
                    retry_after = int(exc.headers.get("Retry-After", "1"))
                    time.sleep(retry_after)
                    continue
                if exc.code >= 500 and attempt + 1 < retries:
                    time.sleep(1 + attempt)
                    continue
                raise last_error
            except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
                last_error = exc
                if attempt + 1 < retries:
                    time.sleep(1 + attempt)
                    continue
                raise
        if last_error:
            raise last_error
        raise RuntimeError(f"Unable to fetch {url}")

    def lookup_show_trakt_id(self, series: SeriesRow) -> int | None:
        if series.trakt:
            return series.trakt

        lookups = []
        if series.imdb:
            lookups.append(f"/search/imdb/{urllib.parse.quote(series.imdb)}?type=show")
        if series.tmdb:
            lookups.append(f"/search/tmdb/{series.tmdb}?type=show")
        if series.tvdb:
            lookups.append(f"/search/tvdb/{series.tvdb}?type=show")

        for path in lookups:
            result = self.get_json(path)
            if isinstance(result, list) and result:
                show = result[0].get("show", {})
                trakt_id = show.get("ids", {}).get("trakt")
                if trakt_id:
                    return int(trakt_id)
        return None

    def get_show_seasons(self, trakt_show_id: int) -> list[dict]:
        result = self.get_json(f"/shows/{trakt_show_id}/seasons?extended=episodes")
        return result if isinstance(result, list) else []

    def get_movie(self, trakt_movie_id: int) -> dict:
        result = self.get_json(f"/movies/{trakt_movie_id}")
        return result if isinstance(result, dict) else {}


def load_tv_library(tv_db: Path) -> tuple[list[SeriesRow], dict[tuple[str, int, int], EpisodeRow]]:
    conn = sqlite3.connect(tv_db)
    conn.row_factory = sqlite3.Row
    try:
        series_rows = [
            SeriesRow(
                id=row["id"],
                name=row["name"] or "",
                imdb=row["imdb"],
                tmdb=row["tmdb"],
                trakt=row["trakt"],
                tvdb=row["tvdb"],
            )
            for row in conn.execute(
                "SELECT id, name, imdb, tmdb, trakt, tvdb FROM series ORDER BY name"
            )
        ]
        episodes = {
            (row["serie_ref"], row["season"], row["number"]): EpisodeRow(
                serie_ref=row["serie_ref"],
                season=row["season"],
                number=row["number"],
                imdb=row["imdb"],
                tmdb=row["tmdb"],
                trakt=row["trakt"],
                tvdb=row["tvdb"],
            )
            for row in conn.execute(
                """
                SELECT serie_ref, season, number, imdb, tmdb, trakt, tvdb
                FROM episodes
                """
            )
        }
        return series_rows, episodes
    finally:
        conn.close()


def load_watched(server_db: Path) -> list[sqlite3.Row]:
    conn = sqlite3.connect(server_db)
    conn.row_factory = sqlite3.Row
    try:
        return list(
            conn.execute(
                "SELECT type, id, user_ref, date, modified FROM Watched ORDER BY type, user_ref, id"
            )
        )
    finally:
        conn.close()


def build_episode_map(
    client: TraktClient,
    series_rows: list[SeriesRow],
    local_episodes: dict[tuple[str, int, int], EpisodeRow],
    out_csv: Path,
) -> tuple[dict[str, str], list[str]]:
    mapping: dict[str, str] = {}
    unresolved_series: list[str] = []
    with out_csv.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=[
                "source",
                "trakt_episode_id",
                "redseat_episode_id",
                "serie_ref",
                "serie_name",
                "serie_imdb",
                "serie_tmdb",
                "serie_tvdb",
                "serie_trakt",
                "season",
                "episode",
                "episode_imdb",
                "episode_tmdb",
                "episode_tvdb",
            ],
        )
        writer.writeheader()

        for episode in local_episodes.values():
            if not episode.trakt:
                continue
            trakt_key = f"trakt:{episode.trakt}"
            mapping[trakt_key] = episode.redseat_id
            writer.writerow(
                {
                    "source": "local_db",
                    "trakt_episode_id": trakt_key,
                    "redseat_episode_id": episode.redseat_id,
                    "serie_ref": episode.serie_ref,
                    "serie_name": "",
                    "serie_imdb": "",
                    "serie_tmdb": "",
                    "serie_tvdb": "",
                    "serie_trakt": "",
                    "season": episode.season,
                    "episode": episode.number,
                    "episode_imdb": episode.imdb or "",
                    "episode_tmdb": episode.tmdb or "",
                    "episode_tvdb": episode.tvdb or "",
                }
            )

        for series in series_rows:
            trakt_show_id = client.lookup_show_trakt_id(series)
            if not trakt_show_id:
                unresolved_series.append(f"{series.id}::{series.name}")
                continue

            try:
                seasons = client.get_show_seasons(trakt_show_id)
            except Exception as exc:  # noqa: BLE001
                unresolved_series.append(f"{series.id}::{series.name}::{exc}")
                continue

            for season in seasons:
                season_number = season.get("number")
                if season_number is None:
                    continue
                for api_episode in season.get("episodes", []):
                    episode_number = api_episode.get("number")
                    trakt_episode_id = api_episode.get("ids", {}).get("trakt")
                    if season_number is None or episode_number is None or not trakt_episode_id:
                        continue

                    local_episode = local_episodes.get((series.id, season_number, episode_number))
                    if not local_episode:
                        continue

                    trakt_key = f"trakt:{trakt_episode_id}"
                    mapping[trakt_key] = local_episode.redseat_id
                    writer.writerow(
                        {
                            "source": "trakt_api",
                            "trakt_episode_id": trakt_key,
                            "redseat_episode_id": local_episode.redseat_id,
                            "serie_ref": series.id,
                            "serie_name": series.name,
                            "serie_imdb": series.imdb or "",
                            "serie_tmdb": series.tmdb or "",
                            "serie_tvdb": series.tvdb or "",
                            "serie_trakt": trakt_show_id,
                            "season": season_number,
                            "episode": episode_number,
                            "episode_imdb": api_episode.get("ids", {}).get("imdb") or "",
                            "episode_tmdb": api_episode.get("ids", {}).get("tmdb") or "",
                            "episode_tvdb": api_episode.get("ids", {}).get("tvdb") or "",
                        }
                    )
    return mapping, unresolved_series


def build_movie_map(
    client: TraktClient,
    watched_rows: list[sqlite3.Row],
    out_csv: Path,
) -> tuple[dict[str, str], list[str]]:
    movie_ids = sorted(
        {
            int(row["id"].split(":", 1)[1])
            for row in watched_rows
            if row["type"] == "movie" and row["id"].startswith("trakt:")
        }
    )
    mapping: dict[str, str] = {}
    unresolved: list[str] = []

    with out_csv.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=["trakt_movie_id", "imdb_movie_id", "title", "year"],
        )
        writer.writeheader()
        for trakt_movie_id in movie_ids:
            try:
                movie = client.get_movie(trakt_movie_id)
            except Exception as exc:  # noqa: BLE001
                unresolved.append(f"{trakt_movie_id}::{exc}")
                continue

            imdb_id = movie.get("ids", {}).get("imdb")
            if not imdb_id:
                unresolved.append(str(trakt_movie_id))
                continue

            trakt_key = f"trakt:{trakt_movie_id}"
            imdb_key = f"imdb:{imdb_id}"
            mapping[trakt_key] = imdb_key
            writer.writerow(
                {
                    "trakt_movie_id": trakt_key,
                    "imdb_movie_id": imdb_key,
                    "title": movie.get("title", ""),
                    "year": movie.get("year", ""),
                }
            )
    return mapping, unresolved


def rewrite_watched(
    server_db: Path,
    episode_map: dict[str, str],
    movie_map: dict[str, str],
) -> dict[str, int]:
    backup_path = server_db.with_name(
        f"{server_db.name}.bak-{time.strftime('%Y%m%d-%H%M%S')}"
    )
    shutil.copy2(server_db, backup_path)

    conn = sqlite3.connect(server_db)
    conn.row_factory = sqlite3.Row
    stats = {
        "backup_created": 1,
        "episodes_updated": 0,
        "episodes_merged": 0,
        "episodes_unresolved": 0,
        "movies_updated": 0,
        "movies_merged": 0,
        "movies_unresolved": 0,
    }

    try:
        rows = list(
            conn.execute(
                "SELECT type, id, user_ref, date, modified FROM Watched ORDER BY type, user_ref, id"
            )
        )
        conn.execute("BEGIN IMMEDIATE")
        for row in rows:
            kind = row["type"]
            old_id = row["id"]
            if kind == "episode":
                new_id = episode_map.get(old_id)
                unresolved_key = "episodes_unresolved"
                updated_key = "episodes_updated"
                merged_key = "episodes_merged"
            elif kind == "movie":
                new_id = movie_map.get(old_id)
                unresolved_key = "movies_unresolved"
                updated_key = "movies_updated"
                merged_key = "movies_merged"
            else:
                continue

            if not new_id or new_id == old_id:
                if old_id.startswith("trakt:"):
                    stats[unresolved_key] += 1
                continue

            destination = conn.execute(
                """
                SELECT date, modified FROM Watched
                WHERE type = ? AND id = ? AND user_ref = ?
                """,
                (kind, new_id, row["user_ref"]),
            ).fetchone()

            if destination:
                merged_date = max(destination["date"], row["date"])
                conn.execute(
                    """
                    UPDATE Watched
                    SET date = ?
                    WHERE type = ? AND id = ? AND user_ref = ?
                    """,
                    (merged_date, kind, new_id, row["user_ref"]),
                )
                conn.execute(
                    """
                    DELETE FROM Watched
                    WHERE type = ? AND id = ? AND user_ref = ?
                    """,
                    (kind, old_id, row["user_ref"]),
                )
                stats[merged_key] += 1
            else:
                conn.execute(
                    """
                    UPDATE Watched
                    SET id = ?
                    WHERE type = ? AND id = ? AND user_ref = ?
                    """,
                    (new_id, kind, old_id, row["user_ref"]),
                )
                stats[updated_key] += 1
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.close()

    stats["backup_path"] = str(backup_path)  # type: ignore[assignment]
    return stats


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Remap Redseat watched entries away from Trakt IDs."
    )
    parser.add_argument("--server-db", required=True, type=Path)
    parser.add_argument("--tv-db", required=True, type=Path)
    parser.add_argument("--client-id", required=True)
    parser.add_argument("--client-secret", default="")
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)
    if args.client_secret:
        print("client_secret provided but not used by this script.", file=sys.stderr)

    watched_rows = load_watched(args.server_db)
    series_rows, local_episodes = load_tv_library(args.tv_db)
    client = TraktClient(args.client_id)

    episode_csv = args.out_dir / "trakt_episode_map.csv"
    movie_csv = args.out_dir / "trakt_movie_map.csv"
    unresolved_series_txt = args.out_dir / "trakt_unresolved_series.txt"
    unresolved_movies_txt = args.out_dir / "trakt_unresolved_movies.txt"
    report_json = args.out_dir / "trakt_remap_report.json"

    episode_map, unresolved_series = build_episode_map(
        client, series_rows, local_episodes, episode_csv
    )
    movie_map, unresolved_movies = build_movie_map(client, watched_rows, movie_csv)
    stats = rewrite_watched(args.server_db, episode_map, movie_map)

    unresolved_series_txt.write_text("\n".join(unresolved_series), encoding="utf-8")
    unresolved_movies_txt.write_text("\n".join(unresolved_movies), encoding="utf-8")

    report = {
        "series_count": len(series_rows),
        "local_episode_count": len(local_episodes),
        "episode_map_count": len(episode_map),
        "movie_map_count": len(movie_map),
        "unresolved_series_count": len(unresolved_series),
        "unresolved_movie_count": len(unresolved_movies),
        "watched_rewrite": stats,
        "files": {
            "episode_csv": str(episode_csv),
            "movie_csv": str(movie_csv),
            "unresolved_series": str(unresolved_series_txt),
            "unresolved_movies": str(unresolved_movies_txt),
        },
    }
    report_json.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
