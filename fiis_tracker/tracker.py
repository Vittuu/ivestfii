from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import List, Optional

from .models import FII, MonthlyRecord
from .storage import DEFAULT_DATA_PATH, load_data, save_data


def normalize_month(value: str) -> str:
    sanitized = value.strip().replace("/", "-")
    if len(sanitized) == 6 and sanitized.isdigit():
        sanitized = f"{sanitized[:4]}-{sanitized[4:]}"
    try:
        dt = datetime.strptime(f"{sanitized}-01", "%Y-%m-%d")
    except ValueError as err:
        raise ValueError("Informe o mes no formato AAAA-MM") from err
    return dt.strftime("%Y-%m")


def month_after(reference: str, offset: int) -> str:
    dt = datetime.strptime(f"{reference}-01", "%Y-%m-%d")
    total_months = dt.year * 12 + dt.month - 1 + offset
    new_year = total_months // 12
    new_month = total_months % 12 + 1
    return f"{new_year:04d}-{new_month:02d}"


@dataclass
class ProjectionPoint:
    month: str
    projected_cotas: float
    projected_income: float
    cumulative_income: float
    reinvested_cotas: float
    combined_cotas: float
    combined_income: float


class FIIsTracker:
    def __init__(self, data_path: Optional[Path] = None) -> None:
        self.data_path = Path(data_path) if data_path else DEFAULT_DATA_PATH
        self._fiis = self._load_fiis()

    def _load_fiis(self) -> List[FII]:
        raw = load_data(self.data_path)
        fiis = [FII.from_dict(item) for item in raw.get("fiis", [])]
        fiis.sort(key=lambda fii: fii.ticker)
        return fiis

    def _save(self) -> None:
        payload = {"fiis": [fii.to_dict() for fii in self._fiis]}
        save_data(payload, self.data_path)

    def refresh(self) -> None:
        self._fiis = self._load_fiis()

    def list_fiis(self) -> List[FII]:
        return list(self._fiis)

    def find_fii(self, ticker: str) -> Optional[FII]:
        ticker_upper = ticker.upper()
        for fii in self._fiis:
            if fii.ticker == ticker_upper:
                return fii
        return None

    def add_or_update_fii(self, ticker: str, name: str, sector: str = "") -> FII:
        existing = self.find_fii(ticker)
        if existing:
            existing.name = name or existing.name
            existing.sector = sector or existing.sector
            self._save()
            return existing
        new_fii = FII(ticker=ticker.upper(), name=name, sector=sector)
        self._fiis.append(new_fii)
        self._fiis.sort(key=lambda fii: fii.ticker)
        self._save()
        return new_fii

    def register_month(self, ticker: str, record: MonthlyRecord) -> MonthlyRecord:
        fii = self.find_fii(ticker)
        if not fii:
            raise ValueError(f"FII {ticker.upper()} nao encontrado.")
        total_cotas_before = fii.total_cotas()
        total_cotas_after = total_cotas_before + record.cotas_added
        if record.dividend_total is None and record.dividend_per_cota:
            record.dividend_total = round(record.dividend_per_cota * total_cotas_after, 2)
        fii.entries.append(record)
        fii.entries.sort(key=lambda entry: entry.month)
        self._save()
        return record

    def update_month_record(self, ticker: str, original_month: str, updated: MonthlyRecord) -> MonthlyRecord:
        fii = self.find_fii(ticker)
        if not fii:
            raise ValueError(f"FII {ticker.upper()} nao encontrado.")
        for idx, entry in enumerate(fii.entries):
            if entry.month == original_month:
                fii.entries[idx] = updated
                break
        else:
            raise ValueError(f"Mes {original_month} nao encontrado para {ticker.upper()}.")
        fii.entries.sort(key=lambda entry: entry.month)
        self._save()
        return updated

    def total_portfolio_dividends(self) -> float:
        return sum(fii.total_dividends_received() for fii in self._fiis)

    def last_updated_at(self) -> Optional[datetime]:
        try:
            timestamp = self.data_path.stat().st_mtime
        except FileNotFoundError:
            return None
        return datetime.fromtimestamp(timestamp)

    def project_income(
        self,
        ticker: str,
        months: int = 12,
        monthly_cotas: float = 1.0,
        window: Optional[int] = 6,
    ) -> List[ProjectionPoint]:
        fii = self.find_fii(ticker)
        if not fii:
            raise ValueError(f"FII {ticker.upper()} nao encontrado.")
        avg_dividend = fii.average_dividend_per_cota(window=window)
        avg_price = fii.average_price()
        if avg_price <= 0:
            avg_price = 1.0
        current_cotas = fii.total_cotas()
        last_month = fii.last_record().month if fii.last_record() else datetime.now().strftime("%Y-%m")
        points: List[ProjectionPoint] = []
        cumulative = 0.0
        cash_remainder = 0.0
        reinvested_total = 0.0
        for offset in range(1, months + 1):
            current_cotas += monthly_cotas
            cotas_before_reinvest = current_cotas
            projected_income = round(cotas_before_reinvest * avg_dividend, 2)
            cumulative = round(cumulative + projected_income, 2)
            cash_remainder = round(cash_remainder + projected_income, 2)
            extra_cotas = 0.0
            if avg_price:
                purchasable = int(cash_remainder // avg_price)
                if purchasable:
                    extra_cotas = float(purchasable)
                    reinvested_total = round(reinvested_total + extra_cotas, 4)
                    current_cotas += extra_cotas
                    cash_remainder = round(cash_remainder - extra_cotas * avg_price, 2)
            combined_cotas = current_cotas
            combined_income = round(combined_cotas * avg_dividend, 2)
            points.append(
                ProjectionPoint(
                    month=month_after(last_month, offset),
                    projected_cotas=cotas_before_reinvest,
                    projected_income=projected_income,
                    cumulative_income=cumulative,
                    reinvested_cotas=reinvested_total,
                    combined_cotas=combined_cotas,
                    combined_income=combined_income,
                )
            )
        return points

    def project_portfolio(
        self,
        months: int = 12,
        monthly_plan: Optional[dict[str, float]] = None,
        window: Optional[int] = 6,
    ) -> List[ProjectionPoint]:
        fiis = self.list_fiis()
        if not fiis:
            return []
        monthly_plan = {ticker.upper(): value for ticker, value in (monthly_plan or {}).items()}
        reference_month = max(
            (fii.last_record().month for fii in fiis if fii.last_record()),
            default=datetime.now().strftime("%Y-%m"),
        )
        state = {
            fii.ticker: {
                "cotas": fii.total_cotas(),
                "monthly_add": monthly_plan.get(fii.ticker, 0.0),
                "avg_dividend": fii.average_dividend_per_cota(window=window),
            }
            for fii in fiis
        }
        points: List[ProjectionPoint] = []
        cumulative = 0.0
        for offset in range(1, months + 1):
            month_income = 0.0
            total_cotas = 0.0
            for ticker, data in state.items():
                data["cotas"] += data["monthly_add"]
                income = data["cotas"] * data["avg_dividend"]
                month_income += income
                total_cotas += data["cotas"]
            month_income = round(month_income, 2)
            cumulative = round(cumulative + month_income, 2)
            points.append(
                ProjectionPoint(
                    month=month_after(reference_month, offset),
                    projected_cotas=total_cotas,
                    projected_income=month_income,
                    cumulative_income=cumulative,
                    reinvested_cotas=0.0,
                    combined_cotas=total_cotas,
                    combined_income=month_income,
                )
            )
        return points

