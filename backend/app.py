from __future__ import annotations

import os
from datetime import datetime, time, timedelta
from typing import List, Optional

import requests
from flask import Flask, jsonify, request
from flask_migrate import Migrate
from flask_sqlalchemy import SQLAlchemy
from werkzeug.security import check_password_hash, generate_password_hash

db = SQLAlchemy()
migrate = Migrate()


class User(db.Model):
    __tablename__ = "users"
    id = db.Column(db.Integer, primary_key=True)
    email = db.Column(db.String(255), unique=True, nullable=False)
    password_hash = db.Column(db.String(255), nullable=False)
    created_at = db.Column(db.DateTime, default=datetime.utcnow, nullable=False)

    def set_password(self, password: str) -> None:
        self.password_hash = generate_password_hash(password)

    def check_password(self, password: str) -> bool:
        return check_password_hash(self.password_hash, password)


class FIICatalog(db.Model):
    __tablename__ = "fiis_catalog"
    ticker = db.Column(db.String(16), primary_key=True)
    name = db.Column(db.String(255), nullable=False)
    sector = db.Column(db.String(255), nullable=True)


class UserFII(db.Model):
    __tablename__ = "user_fiis"
    id = db.Column(db.Integer, primary_key=True)
    user_id = db.Column(db.Integer, db.ForeignKey("users.id", ondelete="CASCADE"), nullable=False)
    ticker = db.Column(db.String(16), db.ForeignKey("fiis_catalog.ticker", ondelete="CASCADE"), nullable=False)
    cotas = db.Column(db.Float, default=0.0)
    avg_price = db.Column(db.Float, default=0.0)
    created_at = db.Column(db.DateTime, default=datetime.utcnow, nullable=False)

    user = db.relationship(User, backref=db.backref("fiis", lazy=True))
    fii = db.relationship(FIICatalog)


class Entry(db.Model):
    __tablename__ = "entries"
    id = db.Column(db.Integer, primary_key=True)
    user_fii_id = db.Column(db.Integer, db.ForeignKey("user_fiis.id", ondelete="CASCADE"), nullable=False)
    month = db.Column(db.String(7), nullable=False)  # YYYY-MM
    cotas_added = db.Column(db.Float, default=0.0)
    price_per_cota = db.Column(db.Float, default=0.0)
    dividend_per_cota = db.Column(db.Float, default=0.0)
    dividend_total = db.Column(db.Float, nullable=True)
    notes = db.Column(db.Text, nullable=True)
    created_at = db.Column(db.DateTime, default=datetime.utcnow, nullable=False)

    user_fii = db.relationship(UserFII, backref=db.backref("entries", lazy=True))


class Quote(db.Model):
    __tablename__ = "quotes"
    id = db.Column(db.Integer, primary_key=True)
    ticker = db.Column(db.String(16), db.ForeignKey("fiis_catalog.ticker"), nullable=False)
    price = db.Column(db.Float, nullable=True)
    dividend_yield = db.Column(db.Float, nullable=True)
    variation = db.Column(db.Float, nullable=True)
    fetched_at = db.Column(db.DateTime, default=datetime.utcnow, nullable=False)

    fii = db.relationship(FIICatalog)


def create_app() -> Flask:
    app = Flask(__name__)
    default_db = "sqlite:///fiis_tracker.db"
    app.config["SQLALCHEMY_DATABASE_URI"] = os.environ.get("DATABASE_URL", default_db)
    app.config["SQLALCHEMY_TRACK_MODIFICATIONS"] = False

    db.init_app(app)
    migrate.init_app(app, db)

    register_routes(app)
    return app


def require_token(req: request) -> bool:
    token = req.headers.get("X-Import-Token")
    expected = os.environ.get("IMPORT_TOKEN", "devtoken")
    return token == expected


def is_market_open(reference: Optional[datetime] = None) -> bool:
    now_utc = reference or datetime.utcnow()
    br_time = now_utc - timedelta(hours=3)  # approximate BRT
    if br_time.weekday() >= 5:
        return False
    start = time(10, 0)
    end = time(18, 30)
    return start <= br_time.time() <= end


def fetch_quote_from_api(ticker: str) -> Optional[dict]:
    api_key = os.environ.get("HGBRASIL_KEY")
    if not api_key:
        return None
    url = "https://api.hgbrasil.com/finance/stock_price"
    params = {"key": api_key, "symbol": ticker}
    resp = requests.get(url, params=params, timeout=10)
    if resp.status_code != 200:
        return None
    payload = resp.json()
    data = payload.get("results", {}).get(ticker.upper())
    if not data:
        return None
    return {
        "price": data.get("price"),
        "dividend_yield": data.get("dividend_yield"),
        "variation": data.get("change_percent"),
    }


def register_routes(app: Flask) -> None:
    @app.get("/health")
    def health() -> tuple[dict, int]:
        return {"status": "ok", "time": datetime.utcnow().isoformat()}, 200

    @app.post("/api/register")
    def register_user():
        data = request.get_json(silent=True) or {}
        email = data.get("email")
        password = data.get("password")
        if not email or not password:
            return jsonify({"error": "email and password required"}), 400
        if User.query.filter_by(email=email).first():
            return jsonify({"error": "user already exists"}), 409
        user = User(email=email)
        user.set_password(password)
        db.session.add(user)
        db.session.commit()
        return jsonify({"status": "ok", "user_id": user.id}), 201

    @app.post("/api/login")
    def login_user():
        data = request.get_json(silent=True) or {}
        email = data.get("email")
        password = data.get("password")
        user = User.query.filter_by(email=email).first()
        if not user or not user.check_password(password or ""):
            return jsonify({"error": "invalid credentials"}), 401
        return jsonify({"status": "ok", "user_id": user.id}), 200

    @app.post("/api/import")
    def import_data():
        if not require_token(request):
            return jsonify({"error": "unauthorized"}), 401

        payload = request.get_json(silent=True)
        if not payload or "fiis" not in payload:
            return jsonify({"error": "invalid payload"}), 400

        user_email = payload.get("user_email", "default@local")
        user = User.query.filter_by(email=user_email).first()
        if not user:
            user = User(email=user_email)
            # define uma senha padrao para importacao, pois password_hash e obrigatorio
            import_password = os.environ.get("IMPORT_DEFAULT_PASSWORD", "importpass")
            user.set_password(import_password)
            db.session.add(user)
            db.session.flush()

        imported = []
        for fii_data in payload["fiis"]:
            ticker = str(fii_data.get("ticker", "")).upper()
            name = fii_data.get("name") or ticker
            sector = fii_data.get("sector")
            if not ticker:
                continue

            catalog = FIICatalog.query.get(ticker)
            if not catalog:
                catalog = FIICatalog(ticker=ticker, name=name, sector=sector)
                db.session.add(catalog)

            user_fii = UserFII.query.filter_by(user_id=user.id, ticker=ticker).first()
            if not user_fii:
                user_fii = UserFII(user_id=user.id, ticker=ticker)
                db.session.add(user_fii)
                db.session.flush()

            entries: List[dict] = fii_data.get("entries", [])
            for entry_data in entries:
                month = entry_data.get("month")
                if not month:
                    continue
                existing = Entry.query.filter_by(user_fii_id=user_fii.id, month=month).first()
                if existing:
                    existing.cotas_added = entry_data.get("cotas_added", existing.cotas_added)
                    existing.price_per_cota = entry_data.get("price_per_cota", existing.price_per_cota)
                    existing.dividend_per_cota = entry_data.get("dividend_per_cota", existing.dividend_per_cota)
                    existing.dividend_total = entry_data.get("dividend_total", existing.dividend_total)
                    existing.notes = entry_data.get("notes", existing.notes)
                else:
                    db.session.add(
                        Entry(
                            user_fii_id=user_fii.id,
                            month=month,
                            cotas_added=entry_data.get("cotas_added", 0.0),
                            price_per_cota=entry_data.get("price_per_cota", 0.0),
                            dividend_per_cota=entry_data.get("dividend_per_cota", 0.0),
                            dividend_total=entry_data.get("dividend_total"),
                            notes=entry_data.get("notes"),
                        )
                    )

            # Atualiza metrica basica (cotas totais e preco medio)
            total_cotas = sum(e.get("cotas_added", 0.0) for e in entries)
            invested = sum(e.get("cotas_added", 0.0) * e.get("price_per_cota", 0.0) for e in entries)
            user_fii.cotas = total_cotas
            user_fii.avg_price = (invested / total_cotas) if total_cotas else 0.0
            imported.append({"ticker": ticker, "entries": len(entries)})

        db.session.commit()
        return jsonify({"status": "ok", "imported": imported}), 201

    @app.get("/api/quotes/<string:ticker>")
    def get_quote(ticker: str):
        ticker = ticker.upper()
        quote = (
            Quote.query.filter_by(ticker=ticker)
            .order_by(Quote.fetched_at.desc())
            .first()
        )
        if not quote:
            return jsonify({"error": "quote not found"}), 404
        return jsonify(
            {
                "ticker": ticker,
                "price": quote.price,
                "dividend_yield": quote.dividend_yield,
                "variation": quote.variation,
                "fetched_at": quote.fetched_at.isoformat(),
            }
        )

    @app.post("/api/quotes/<string:ticker>/refresh")
    def refresh_quote(ticker: str):
        ticker = ticker.upper()
        latest = (
            Quote.query.filter_by(ticker=ticker)
            .order_by(Quote.fetched_at.desc())
            .first()
        )
        if latest and (datetime.utcnow() - latest.fetched_at).total_seconds() < 3600:
            return jsonify(
                {
                    "ticker": ticker,
                    "price": latest.price,
                    "dividend_yield": latest.dividend_yield,
                    "variation": latest.variation,
                    "fetched_at": latest.fetched_at.isoformat(),
                    "source": "cache",
                }
            )
        if not is_market_open() and latest:
            return jsonify(
                {
                    "ticker": ticker,
                    "price": latest.price,
                    "dividend_yield": latest.dividend_yield,
                    "variation": latest.variation,
                    "fetched_at": latest.fetched_at.isoformat(),
                    "source": "outside_market",
                }
            )
        fetched = fetch_quote_from_api(ticker)
        if not fetched:
            return jsonify({"error": "quote unavailable"}), 503
        quote = Quote(
            ticker=ticker,
            price=fetched.get("price"),
            dividend_yield=fetched.get("dividend_yield"),
            variation=fetched.get("variation"),
            fetched_at=datetime.utcnow(),
        )
        db.session.add(quote)
        db.session.commit()
        return jsonify(
            {
                "ticker": ticker,
                "price": quote.price,
                "dividend_yield": quote.dividend_yield,
                "variation": quote.variation,
                "fetched_at": quote.fetched_at.isoformat(),
                "source": "api",
            }
        )


app = create_app()

if __name__ == "__main__":
    app.run(debug=True)
