from __future__ import annotations

from dataclasses import dataclass, field
from statistics import mean
from typing import Any, Dict, List, Optional


@dataclass
class MonthlyRecord:
    month: str  # YYYY-MM
    cotas_added: float = 0.0
    price_per_cota: float = 0.0
    dividend_per_cota: float = 0.0
    dividend_total: Optional[float] = None
    notes: str = ""

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "MonthlyRecord":
        return cls(
            month=data.get("month", ""),
            cotas_added=float(data.get("cotas_added", 0.0)),
            price_per_cota=float(data.get("price_per_cota", 0.0)),
            dividend_per_cota=float(data.get("dividend_per_cota", 0.0)),
            dividend_total=(
                float(data["dividend_total"]) if data.get("dividend_total") is not None else None
            ),
            notes=data.get("notes", ""),
        )

    def to_dict(self) -> Dict[str, Any]:
        return {
            "month": self.month,
            "cotas_added": self.cotas_added,
            "price_per_cota": self.price_per_cota,
            "dividend_per_cota": self.dividend_per_cota,
            "dividend_total": self.dividend_total,
            "notes": self.notes,
        }


@dataclass
class FII:
    ticker: str
    name: str
    sector: str = ""
    entries: List[MonthlyRecord] = field(default_factory=list)

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "FII":
        entries = [MonthlyRecord.from_dict(item) for item in data.get("entries", [])]
        entries.sort(key=lambda record: record.month)
        return cls(
            ticker=data.get("ticker", "").upper(),
            name=data.get("name", ""),
            sector=data.get("sector", ""),
            entries=entries,
        )

    def to_dict(self) -> Dict[str, Any]:
        return {
            "ticker": self.ticker.upper(),
            "name": self.name,
            "sector": self.sector,
            "entries": [entry.to_dict() for entry in sorted(self.entries, key=lambda record: record.month)],
        }

    def total_cotas(self) -> float:
        return sum(entry.cotas_added for entry in self.entries)

    def total_invested(self) -> float:
        return sum(entry.cotas_added * entry.price_per_cota for entry in self.entries)

    def average_price(self) -> float:
        cotas = self.total_cotas()
        return self.total_invested() / cotas if cotas else 0.0

    def average_dividend_per_cota(self, window: Optional[int] = None) -> float:
        dividends = [entry.dividend_per_cota for entry in self.entries if entry.dividend_per_cota > 0]
        if not dividends:
            return 0.0
        if window:
            dividends = dividends[-window:]
        return mean(dividends)

    def last_record(self) -> Optional[MonthlyRecord]:
        if not self.entries:
            return None
        return sorted(self.entries, key=lambda entry: entry.month)[-1]

    def total_dividends_received(self) -> float:
        total = 0.0
        for entry in self.entries:
            if entry.dividend_total is not None:
                total += entry.dividend_total
            elif entry.dividend_per_cota and entry.cotas_added:
                total += entry.dividend_per_cota * entry.cotas_added
        return total
