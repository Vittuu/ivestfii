from __future__ import annotations

from datetime import datetime
import customtkinter as ctk
from tkinter import Canvas, messagebox, ttk
from typing import Callable, Optional

from .models import MonthlyRecord
from .tracker import FIIsTracker, normalize_month


def parse_float(value: str, default: float = 0.0) -> float:
    cleaned = value.strip().replace(",", ".")
    if not cleaned:
        return default
    return float(cleaned)


class BaseModal(ctk.CTkToplevel):
    def __init__(self, master: ctk.CTk | ctk.CTkToplevel, title: str, width: int = 420, height: int = 420) -> None:
        super().__init__(master)
        self.title(title)
        self.geometry(f"{width}x{height}+{master.winfo_x()+50}+{master.winfo_y()+50}")
        self.transient(master)
        self.grab_set()
        self.resizable(False, False)
        self.columnconfigure(0, weight=1)


class FiiModal(BaseModal):
    def __init__(
        self,
        master: ctk.CTk,
        tracker: FIIsTracker,
        callback: Callable[[Optional[str]], None],
        ticker: Optional[str] = None,
    ) -> None:
        super().__init__(master, "FII", width=420, height=320)
        self.tracker = tracker
        self.callback = callback
        self.fii = tracker.find_fii(ticker) if ticker else None

        self.label = ctk.CTkLabel(self, text="Cadastrar/editar FII", font=("Segoe UI", 16, "bold"))
        self.label.grid(row=0, column=0, pady=(15, 10))

        self.entry_ticker = ctk.CTkEntry(self, placeholder_text="Ticker (ex: KNRI11)")
        if self.fii:
            self.entry_ticker.insert(0, self.fii.ticker)
        self.entry_ticker.grid(row=1, column=0, padx=25, pady=6, sticky="ew")

        self.entry_name = ctk.CTkEntry(self, placeholder_text="Nome")
        if self.fii:
            self.entry_name.insert(0, self.fii.name)
        self.entry_name.grid(row=2, column=0, padx=25, pady=6, sticky="ew")

        self.entry_sector = ctk.CTkEntry(self, placeholder_text="Setor")
        if self.fii:
            self.entry_sector.insert(0, self.fii.sector)
        self.entry_sector.grid(row=3, column=0, padx=25, pady=6, sticky="ew")

        self.button_save = ctk.CTkButton(self, text="Salvar", command=self.save)
        self.button_save.grid(row=4, column=0, padx=25, pady=(15, 12), sticky="ew")

    def save(self) -> None:
        ticker = self.entry_ticker.get().strip().upper()
        name = self.entry_name.get().strip()
        sector = self.entry_sector.get().strip()

        if not ticker or not name:
            messagebox.showwarning("Campos obrigatorios", "Informe o ticker e o nome.")
            return
        self.tracker.add_or_update_fii(ticker, name, sector)
        self.callback(ticker)
        self.destroy()


class MonthModal(BaseModal):
    def __init__(
        self,
        master: ctk.CTk,
        tracker: FIIsTracker,
        ticker: str,
        callback: Callable[[Optional[str]], None],
    ) -> None:
        super().__init__(master, "Registro mensal", width=460, height=520)
        self.tracker = tracker
        self.ticker = ticker
        self.callback = callback
        fii = tracker.find_fii(ticker)
        if not fii:
            messagebox.showerror("Erro", "FII nao encontrado.")
            self.destroy()
            return

        fields = [
            ("Mes (AAAA-MM)", "field_month"),
            ("Cotas compradas", "field_cotas"),
            ("Preco por cota", "field_price"),
            ("Dividendo por cota", "field_divpc"),
            ("Dividendo total (opcional)", "field_divtotal"),
            ("Observacoes", "field_notes"),
        ]
        self.inputs: dict[str, ctk.CTkEntry | ctk.CTkTextbox] = {}

        ctk.CTkLabel(self, text=f"Lancamento para {ticker}", font=("Segoe UI", 16, "bold")).grid(
            row=0, column=0, pady=(15, 10)
        )

        for idx, (label, key) in enumerate(fields, start=1):
            ctk.CTkLabel(self, text=label).grid(row=idx * 2 - 1, column=0, sticky="w", padx=25)
            if key == "field_notes":
                widget = ctk.CTkTextbox(self, height=70)
            else:
                widget = ctk.CTkEntry(self)
                if key == "field_cotas":
                    widget.insert(0, "0")
                if key == "field_price":
                    widget.insert(0, f"{fii.average_price():.2f}")
                if key == "field_divpc":
                    widget.insert(0, "0")
            widget.grid(row=idx * 2, column=0, padx=25, pady=(0, 8), sticky="ew")
            self.inputs[key] = widget

        ctk.CTkButton(self, text="Salvar", command=self.save).grid(row=20, column=0, padx=25, pady=(10, 15), sticky="ew")

    def save(self) -> None:
        try:
            month = normalize_month(self.inputs["field_month"].get())
        except ValueError as err:
            messagebox.showwarning("Mes invalido", str(err))
            return
        cotas = parse_float(self.inputs["field_cotas"].get(), 0.0)
        price = parse_float(self.inputs["field_price"].get(), 0.0)
        dividend_pc = parse_float(self.inputs["field_divpc"].get(), 0.0)
        div_total_clean = self.inputs["field_divtotal"].get().strip()
        dividend_total = parse_float(div_total_clean) if div_total_clean else None
        notes_widget = self.inputs["field_notes"]
        notes = notes_widget.get("1.0", "end").strip() if isinstance(notes_widget, ctk.CTkTextbox) else ""

        record = MonthlyRecord(
            month=month,
            cotas_added=cotas,
            price_per_cota=price,
            dividend_per_cota=dividend_pc,
            dividend_total=dividend_total,
            notes=notes,
        )
        self.tracker.register_month(self.ticker, record)
        self.callback(self.ticker)
        self.destroy()


class HistoryModal(BaseModal):
    def __init__(self, master: ctk.CTk, tracker: FIIsTracker, ticker: str) -> None:
        super().__init__(master, f"Historico {ticker}", width=520, height=500)
        fii = tracker.find_fii(ticker)
        text = "Sem lancamentos." if not fii or not fii.entries else "\n".join(
            f"{entry.month}  +{entry.cotas_added:.2f} @ R$ {entry.price_per_cota:.2f}  "
            f"Div/cota R$ {entry.dividend_per_cota:.2f}  Total R$ {(entry.dividend_total or 0):.2f}"
            + (f"  {entry.notes}" if entry.notes else "")
            for entry in sorted(fii.entries, key=lambda item: item.month)
        )
        box = ctk.CTkTextbox(self, height=420, width=480)
        box.insert("end", text)
        box.configure(state="disabled")
        box.grid(row=0, column=0, padx=20, pady=20, sticky="nsew")


class EditHistoryModal(BaseModal):
    """Allow editing an existing monthly record."""

    def __init__(
        self,
        master: ctk.CTk,
        tracker: FIIsTracker,
        ticker: str,
        callback: Callable[[Optional[str]], None],
    ) -> None:
        super().__init__(master, f"Editar historico {ticker}", width=520, height=640)
        self.tracker = tracker
        self.ticker = ticker
        self.callback = callback
        fii = tracker.find_fii(ticker)
        self.records = sorted(fii.entries, key=lambda item: item.month) if fii else []
        if not self.records:
            messagebox.showinfo("Sem dados", "Ainda nao ha lancamentos para editar.")
            self.destroy()
            return

        self.selected_month = ctk.StringVar(value=self.records[0].month)
        ctk.CTkLabel(self, text=f"Editar {ticker}", font=("Segoe UI", 16, "bold")).grid(row=0, column=0, pady=(10, 8))
        self.selector = ctk.CTkOptionMenu(
            self,
            values=[record.month for record in self.records],
            variable=self.selected_month,
            command=self._load_record,
        )
        self.selector.grid(row=1, column=0, padx=25, pady=(0, 10), sticky="ew")

        self.field_entries: dict[str, ctk.CTkEntry] = {}
        specs = [
            ("Mes (AAAA-MM)", "month"),
            ("Cotas compradas", "cotas"),
            ("Preco por cota", "price"),
            ("Dividendo por cota", "div_pc"),
            ("Dividendo total (opcional)", "div_total"),
        ]
        current_row = 2
        for label, key in specs:
            ctk.CTkLabel(self, text=label).grid(row=current_row, column=0, padx=25, pady=(4, 0), sticky="w")
            entry = ctk.CTkEntry(self)
            entry.grid(row=current_row + 1, column=0, padx=25, pady=(0, 6), sticky="ew")
            self.field_entries[key] = entry
            current_row += 2

        ctk.CTkLabel(self, text="Observacoes").grid(row=current_row, column=0, padx=25, pady=(8, 4), sticky="w")
        self.notes_text = ctk.CTkTextbox(self, height=140)
        self.notes_text.grid(row=current_row + 1, column=0, padx=25, pady=(0, 12), sticky="nsew")

        ctk.CTkButton(self, text="Salvar alteracoes", command=self.save).grid(
            row=current_row + 2, column=0, padx=25, pady=(6, 16), sticky="ew"
        )

        self.current_month_key = self.selected_month.get()
        self._load_record(self.current_month_key)

    def _load_record(self, month: str) -> None:
        record = next((item for item in self.records if item.month == month), None)
        if not record:
            return
        self.current_month_key = record.month
        self.field_entries["month"].delete(0, "end")
        self.field_entries["month"].insert(0, record.month)
        self.field_entries["cotas"].delete(0, "end")
        self.field_entries["cotas"].insert(0, f"{record.cotas_added:.2f}")
        self.field_entries["price"].delete(0, "end")
        self.field_entries["price"].insert(0, f"{record.price_per_cota:.2f}")
        self.field_entries["div_pc"].delete(0, "end")
        self.field_entries["div_pc"].insert(0, f"{record.dividend_per_cota:.2f}")
        self.field_entries["div_total"].delete(0, "end")
        if record.dividend_total is not None:
            self.field_entries["div_total"].insert(0, f"{record.dividend_total:.2f}")
        self.notes_text.delete("1.0", "end")
        if record.notes:
            self.notes_text.insert("end", record.notes)

    def save(self) -> None:
        try:
            month_value = normalize_month(self.field_entries["month"].get())
        except ValueError as err:
            messagebox.showwarning("Mes invalido", str(err))
            return
        cotas = parse_float(self.field_entries["cotas"].get() or "0", 0.0)
        price = parse_float(self.field_entries["price"].get() or "0", 0.0)
        div_pc = parse_float(self.field_entries["div_pc"].get() or "0", 0.0)
        div_total_raw = self.field_entries["div_total"].get().strip()
        div_total = parse_float(div_total_raw, 0.0) if div_total_raw else None
        notes = self.notes_text.get("1.0", "end").strip()

        record = MonthlyRecord(
            month=month_value,
            cotas_added=cotas,
            price_per_cota=price,
            dividend_per_cota=div_pc,
            dividend_total=div_total,
            notes=notes,
        )
        try:
            self.tracker.update_month_record(self.ticker, self.current_month_key, record)
        except ValueError as err:
            messagebox.showerror("Erro", str(err))
            return
        self.callback(self.ticker)
        self.destroy()

class ProjectionModal(BaseModal):
    def __init__(self, master: ctk.CTk, tracker: FIIsTracker, ticker: str) -> None:
        super().__init__(master, f"Projecao {ticker}", width=520, height=520)
        self.tracker = tracker
        self.ticker = ticker

        self.entry_months = ctk.CTkEntry(self, placeholder_text="Meses (padrao 12)")
        self.entry_months.grid(row=0, column=0, padx=25, pady=(20, 8), sticky="ew")

        self.entry_cotas = ctk.CTkEntry(self, placeholder_text="Cotas adicionais/m (padrao 1)")
        self.entry_cotas.grid(row=1, column=0, padx=25, pady=8, sticky="ew")

        self.entry_window = ctk.CTkEntry(self, placeholder_text="Janela media de dividendos (enter = tudo)")
        self.entry_window.grid(row=2, column=0, padx=25, pady=8, sticky="ew")

        ctk.CTkButton(self, text="Calcular", command=self.calculate).grid(row=3, column=0, padx=25, pady=12, sticky="ew")

        self.output = ctk.CTkTextbox(self, height=320)
        self.output.grid(row=4, column=0, padx=25, pady=(0, 20), sticky="nsew")
        self.output.configure(state="disabled")

    def calculate(self) -> None:
        months = int(self.entry_months.get() or "12")
        monthly = parse_float(self.entry_cotas.get() or "1", 1.0)
        window_value = self.entry_window.get().strip()
        window = int(window_value) if window_value else None
        points = self.tracker.project_income(self.ticker, months=months, monthly_cotas=monthly, window=window)
        text = "Sem dados de dividendos." if not points else "\n".join(
            f"{p.month}: {p.projected_cotas:.2f} cotas -> R$ {p.projected_income:.2f}" for p in points
        )
        self.output.configure(state="normal")
        self.output.delete("1.0", "end")
        self.output.insert("end", text)
        self.output.configure(state="disabled")


class TrackerWindow(ctk.CTk):
    def __init__(self, data_path: Optional[str] = None) -> None:
        super().__init__()
        ctk.set_appearance_mode("dark")
        ctk.set_default_color_theme("green")
        self.title("FIIs Tracker")
        self.geometry("1280x760")
        self.minsize(1100, 640)
        self.configure(fg_color="#05070f")

        self.tracker = FIIsTracker(data_path=data_path)
        self.active_ticker: Optional[str] = None
        self.portfolio_entries: dict[str, ctk.CTkEntry] = {}
        self.proj_fii_var = ctk.StringVar(value="-")
        self._table_bg = "#1f2937"
        self._table_alt_bg = "#273347"

        self.style = ttk.Style()
        self.style.theme_use("default")
        self._apply_table_colors()

        self.gradient_canvas = Canvas(self, highlightthickness=0, bd=0, bg=self.cget("fg_color"))
        self.gradient_canvas.place(x=0, y=0, relwidth=1, relheight=1)
        self.gradient_canvas.tk.call("lower", self.gradient_canvas._w)
        self.bind("<Configure>", self._on_resize)
        self._draw_gradient()

        self._build_layout()
        self.refresh_all()
        self._update_clock()

    def _apply_table_colors(self) -> None:
        bg = "#1f2937"
        alt_bg = "#273347"
        fg = "#e2e8f0"
        heading_bg = "#111827"
        heading_fg = "#f8fafc"
        border = "#334155"
        self.style.configure(
            "Treeview",
            background=bg,
            fieldbackground=bg,
            foreground=fg,
            rowheight=28,
            bordercolor=border,
            borderwidth=0,
        )
        self.style.map("Treeview", background=[("selected", "#2563eb")])
        self.style.configure(
            "Treeview.Heading",
            background=heading_bg,
            foreground=heading_fg,
            relief="flat",
        )
        self.style.map("Treeview.Heading", background=[("active", heading_bg)])
        self.style.configure("Treeview", font=("Segoe UI", 11))
        self.style.configure("Treeview.Heading", font=("Segoe UI", 11, "bold"))

    def _on_resize(self, event: object) -> None:
        self._draw_gradient()

    def _draw_gradient(self) -> None:
        canvas = self.gradient_canvas
        canvas.delete("gradient")
        width = max(canvas.winfo_width(), 1)
        height = max(canvas.winfo_height(), 1)
        start = (15, 23, 42)  # #0f172a
        end = (31, 41, 55)  # #1f2937
        steps = 120
        for i in range(steps):
            ratio = i / steps
            r = int(start[0] + (end[0] - start[0]) * ratio)
            g = int(start[1] + (end[1] - start[1]) * ratio)
            b = int(start[2] + (end[2] - start[2]) * ratio)
            color = f"#{r:02x}{g:02x}{b:02x}"
            y1 = int(i * height / steps)
            y2 = int((i + 1) * height / steps)
            canvas.create_rectangle(0, y1, width, y2, outline="", fill=color, tags="gradient")


    def _build_layout(self) -> None:
        self.rowconfigure(0, weight=1)
        self.rowconfigure(1, weight=0)
        self.columnconfigure(0, weight=1)

        self.tabview = ctk.CTkTabview(self)
        self.tabview.grid(row=0, column=0, sticky="nsew", padx=20, pady=(20, 10))
        for name in ("Dashboard", "Gerenciar", "Projecoes"):
            self.tabview.add(name)

        self._build_dashboard_tab()
        self._build_manage_tab()
        self._build_projection_tab()

        self.info_frame = ctk.CTkFrame(self, corner_radius=0)
        self.info_frame.grid(row=1, column=0, sticky="ew", padx=0, pady=(0, 10))
        self.info_frame.grid_columnconfigure((0, 1, 2), weight=1, uniform="info")
        label_kwargs = {"font": ("Segoe UI", 14), "anchor": "center", "justify": "center"}
        self.clock_label = ctk.CTkLabel(self.info_frame, text="Agora: --", **label_kwargs)
        self.clock_label.grid(row=0, column=0, padx=20, pady=8, sticky="ew")
        self.last_update_label = ctk.CTkLabel(self.info_frame, text="Ultima atualizacao: --", **label_kwargs)
        self.last_update_label.grid(row=0, column=1, padx=20, pady=8, sticky="ew")
        self.dividends_label = ctk.CTkLabel(self.info_frame, text="Total recebido: R$ 0,00", **label_kwargs)
        self.dividends_label.grid(row=0, column=2, padx=20, pady=8, sticky="ew")

    # Dashboard ----------------------------------------------------------------
    def _build_dashboard_tab(self) -> None:
        tab = self.tabview.tab("Dashboard")
        tab.grid_columnconfigure(0, weight=1)
        tab.grid_rowconfigure(1, weight=1)

        self.dashboard_metrics = ctk.CTkFrame(tab, fg_color="transparent")
        self.dashboard_metrics.grid(row=0, column=0, sticky="ew", padx=10, pady=(15, 10))
        self.dashboard_metrics.grid_columnconfigure((0, 1, 2, 3), weight=1, uniform="metrics")
        self.metric_value_labels: dict[str, ctk.CTkLabel] = {}
        card_titles = [
            ("Investido total", "invested"),
            ("Renda mensal estimada", "income"),
            ("DY medio", "dy"),
            ("Total recebido", "dividends"),
        ]
        for idx, (title, key) in enumerate(card_titles):
            card = ctk.CTkFrame(self.dashboard_metrics, corner_radius=18, fg_color="#111827")
            card.grid(row=0, column=idx, padx=6, ipadx=5, ipady=5, sticky="nsew")
            ctk.CTkLabel(card, text=title, font=("Segoe UI", 12, "bold"), text_color="#94a3b8").pack(
                anchor="w", padx=16, pady=(12, 4)
            )
            value_label = ctk.CTkLabel(card, text="--", font=("Segoe UI", 20, "bold"))
            value_label.pack(anchor="w", padx=16, pady=(0, 14))
            self.metric_value_labels[key] = value_label

        self.dashboard_cards = ctk.CTkScrollableFrame(tab, corner_radius=16, border_width=1, fg_color="#0f172a")
        self.dashboard_cards.grid(row=1, column=0, sticky="nsew", padx=10, pady=(0, 10))
        self.dashboard_cards.grid_columnconfigure((0, 1), weight=1)

    def _update_clock(self) -> None:
        now = datetime.now().strftime("%d/%m/%Y %H:%M")
        if hasattr(self, "clock_label"):
            self.clock_label.configure(text=f"Hoje: {now}")
        self.after(1000, self._update_clock)

    def update_dashboard_info(self) -> None:
        last = self.tracker.last_updated_at()
        last_text = last.strftime("%d/%m/%Y %H:%M:%S") if last else "sem dados"
        self.last_update_label.configure(text=f"Ultima atualizacao: {last_text}")
        total_dividends = self.tracker.total_portfolio_dividends()
        self.dividends_label.configure(text=f"Total recebido: R$ {total_dividends:.2f}")

    def update_dashboard(self) -> None:
        for widget in self.dashboard_cards.winfo_children():
            widget.destroy()

        fiis = self.tracker.list_fiis()
        total_invested = sum(fii.total_invested() for fii in fiis)
        total_income = sum(fii.total_cotas() * fii.average_dividend_per_cota() for fii in fiis)
        total_dividends = self.tracker.total_portfolio_dividends()
        yield_percent = (total_income / total_invested * 100) if total_invested else 0.0
        self.update_dashboard_info()
        metrics_values = {
            "invested": f"R$ {total_invested:.2f}" if fiis else "--",
            "income": f"R$ {total_income:.2f}" if fiis else "--",
            "dy": f"{yield_percent:.2f}%" if fiis else "--",
            "dividends": f"R$ {total_dividends:.2f}" if fiis else "--",
        }
        for key, label in self.metric_value_labels.items():
            label.configure(text=metrics_values.get(key, "--"))

        if not fiis:
            ctk.CTkLabel(self.dashboard_cards, text="Sem dados para exibir.", font=("Segoe UI", 14)).grid(
                row=0, column=0, padx=20, pady=20
            )
            return

        for index, fii in enumerate(fiis):
            card = ctk.CTkFrame(self.dashboard_cards, corner_radius=12, border_width=1)
            row = index // 2
            column = index % 2
            card.grid(row=row, column=column, padx=12, pady=12, sticky="nsew")
            card.grid_columnconfigure(0, weight=1)

            monthly_income = fii.total_cotas() * fii.average_dividend_per_cota()
            invested = fii.total_invested()
            dy = (monthly_income / invested * 100) if invested else 0.0
            last = fii.last_record()

            lines = [
                f"{fii.ticker} - {fii.name}",
                f"Setor: {fii.sector or 'n/d'}",
                f"Cotas: {fii.total_cotas():.2f} | Preco medio: R$ {fii.average_price():.2f}",
                f"Investido: R$ {invested:.2f}",
                f"Dividendo medio/cota: R$ {fii.average_dividend_per_cota():.2f}",
                f"Renda estimada: R$ {monthly_income:.2f}/mes | DY: {dy:.2f}%",
            ]
            if last:
                lines.append(f"Ultimo mes: {last.month} (R$ {(last.dividend_total or 0):.2f})")
            lines.append(f"Total recebido: R$ {fii.total_dividends_received():.2f}")
            ctk.CTkLabel(card, text="\n".join(lines), justify="left").grid(row=0, column=0, padx=12, pady=12, sticky="w")

    # Manage tab ---------------------------------------------------------------
    def _build_manage_tab(self) -> None:
        tab = self.tabview.tab("Gerenciar")
        tab.grid_columnconfigure(0, weight=3)
        tab.grid_columnconfigure(1, weight=2)
        tab.grid_rowconfigure(0, weight=1)

        left_frame = ctk.CTkFrame(tab)
        left_frame.grid(row=0, column=0, sticky="nsew", padx=(10, 10), pady=10)
        left_frame.columnconfigure(0, weight=1)
        left_frame.rowconfigure(0, weight=1)

        columns = ("ticker", "name", "sector", "cotas", "invested", "avg_price", "avg_div", "income")
        self.tree = ttk.Treeview(left_frame, columns=columns, show="headings", height=18)
        headers = {
            "ticker": ("Ticker", 100),
            "name": ("Nome", 160),
            "sector": ("Setor", 120),
            "cotas": ("Cotas", 90),
            "invested": ("Investido", 120),
            "avg_price": ("Preco medio", 110),
            "avg_div": ("Div/cota", 100),
            "income": ("Renda/m", 110),
        }
        for key in columns:
            label, width = headers[key]
            self.tree.heading(key, text=label)
            self.tree.column(key, width=width, anchor="center")
        self.tree.tag_configure("evenrow", background=self._table_bg, foreground="#e2e8f0")
        self.tree.tag_configure("oddrow", background=self._table_alt_bg, foreground="#e2e8f0")
        self.tree.bind("<<TreeviewSelect>>", self.on_tree_select)

        tree_scroll = ttk.Scrollbar(left_frame, orient="vertical", command=self.tree.yview)
        self.tree.configure(yscrollcommand=tree_scroll.set)
        self.tree.grid(row=0, column=0, sticky="nsew")
        tree_scroll.grid(row=0, column=1, sticky="ns")

        right_frame = ctk.CTkFrame(tab)
        right_frame.grid(row=0, column=1, sticky="nsew", padx=(0, 10), pady=10)
        right_frame.columnconfigure(0, weight=1)
        right_frame.rowconfigure(1, weight=1)

        ctk.CTkLabel(right_frame, text="Detalhes", font=("Segoe UI", 18, "bold")).grid(
            row=0, column=0, pady=(15, 8)
        )
        self.details = ctk.CTkTextbox(right_frame, height=240)
        self.details.grid(row=1, column=0, padx=20, pady=(0, 10), sticky="nsew")
        self.details.configure(state="disabled")

        button_frame = ctk.CTkFrame(right_frame, fg_color="transparent")
        button_frame.grid(row=2, column=0, pady=10, padx=20, sticky="ew")
        button_frame.grid_columnconfigure((0, 1), weight=1)

        ctk.CTkButton(button_frame, text="Adicionar FII", command=self.open_add_modal).grid(
            row=0, column=0, padx=6, pady=6, sticky="ew"
        )
        ctk.CTkButton(button_frame, text="Registrar mes", command=self.open_month_modal).grid(
            row=0, column=1, padx=6, pady=6, sticky="ew"
        )
        ctk.CTkButton(button_frame, text="Historico", command=self.open_history_modal).grid(
            row=1, column=0, padx=6, pady=6, sticky="ew"
        )
        ctk.CTkButton(button_frame, text="Projecao individual", command=self.open_projection_modal).grid(
            row=1, column=1, padx=6, pady=6, sticky="ew"
        )
        ctk.CTkButton(button_frame, text="Editar historico", command=self.open_edit_history_modal).grid(
            row=2, column=0, columnspan=2, padx=6, pady=6, sticky="ew"
        )
        ctk.CTkButton(button_frame, text="Recarregar", command=self.reload_data).grid(
            row=3, column=0, columnspan=2, padx=6, pady=(6, 0), sticky="ew"
        )

        self.status = ctk.CTkLabel(right_frame, text=f"Arquivo: {self.tracker.data_path}", anchor="w")
        self.status.grid(row=3, column=0, padx=20, pady=(0, 12), sticky="ew")

    # Projection tab -----------------------------------------------------------
    def _build_projection_tab(self) -> None:
        tab = self.tabview.tab("Projecoes")
        tab.grid_columnconfigure((0, 1), weight=1)
        tab.grid_rowconfigure(0, weight=1)

        # Individual section
        indiv = ctk.CTkFrame(tab)
        indiv.grid(row=0, column=0, sticky="nsew", padx=(10, 5), pady=10)
        indiv.columnconfigure(0, weight=1)

        ctk.CTkLabel(indiv, text="Projecao individual", font=("Segoe UI", 16, "bold")).grid(
            row=0, column=0, pady=(10, 8)
        )
        self.proj_fii_option = ctk.CTkOptionMenu(indiv, values=["-"], variable=self.proj_fii_var)
        self.proj_fii_option.grid(row=1, column=0, padx=20, pady=4, sticky="ew")

        self.proj_months_entry = ctk.CTkEntry(indiv, placeholder_text="Meses (ex.: 12)")
        self.proj_months_entry.grid(row=2, column=0, padx=20, pady=4, sticky="ew")
        self.proj_cotas_entry = ctk.CTkEntry(indiv, placeholder_text="Cotas adicionadas/m (ex.: 1)")
        self.proj_cotas_entry.grid(row=3, column=0, padx=20, pady=4, sticky="ew")
        self.proj_window_entry = ctk.CTkEntry(indiv, placeholder_text="Janela media de dividendos (enter = tudo)")
        self.proj_window_entry.grid(row=4, column=0, padx=20, pady=4, sticky="ew")

        ctk.CTkButton(indiv, text="Calcular", command=self.run_individual_projection).grid(
            row=5, column=0, padx=20, pady=(8, 8), sticky="ew"
        )
        indiv_table_frame = ctk.CTkFrame(indiv, fg_color="transparent")
        indiv_table_frame.grid(row=6, column=0, padx=20, pady=(0, 20), sticky="nsew")
        indiv_table_frame.columnconfigure(0, weight=1)
        indiv_table_frame.rowconfigure(0, weight=1)
        proj_columns = (
            "month",
            "cotas",
            "renda",
            "acumulada",
            "reinvest",
            "total_cotas",
            "renda_total",
        )
        self.proj_tree = ttk.Treeview(
            indiv_table_frame,
            columns=proj_columns,
            show="headings",
            height=12,
        )
        headings = {
            "month": ("Mes", 90),
            "cotas": ("Cotas totais", 110),
            "renda": ("Renda/m", 110),
            "acumulada": ("Renda acum.", 110),
            "reinvest": ("Cotas via renda", 130),
            "total_cotas": ("Cotas totais+", 130),
            "renda_total": ("Renda total/m", 130),
        }
        for key, (title, width) in headings.items():
            self.proj_tree.heading(key, text=title)
            self.proj_tree.column(key, width=width, anchor="center")
        self.proj_tree.tag_configure("evenrow", background=self._table_bg, foreground="#e2e8f0")
        self.proj_tree.tag_configure("oddrow", background=self._table_alt_bg, foreground="#e2e8f0")
        proj_scroll = ttk.Scrollbar(indiv_table_frame, orient="vertical", command=self.proj_tree.yview)
        self.proj_tree.configure(yscrollcommand=proj_scroll.set)
        self.proj_tree.grid(row=0, column=0, sticky="nsew")
        proj_scroll.grid(row=0, column=1, sticky="ns")

        # Portfolio section
        portfolio = ctk.CTkFrame(tab)
        portfolio.grid(row=0, column=1, sticky="nsew", padx=(5, 10), pady=10)
        portfolio.columnconfigure(0, weight=1)
        portfolio.rowconfigure(4, weight=1)

        ctk.CTkLabel(portfolio, text="Projecao consolidada", font=("Segoe UI", 16, "bold")).grid(
            row=0, column=0, pady=(10, 8)
        )
        self.port_months_entry = ctk.CTkEntry(portfolio, placeholder_text="Meses (ex.: 12)")
        self.port_months_entry.grid(row=1, column=0, padx=20, pady=4, sticky="ew")
        self.port_window_entry = ctk.CTkEntry(portfolio, placeholder_text="Janela media de dividendos (enter = tudo)")
        self.port_window_entry.grid(row=2, column=0, padx=20, pady=4, sticky="ew")
        ctk.CTkLabel(portfolio, text="Cotas adicionadas por FII (por mes)").grid(row=3, column=0, padx=20, pady=4)

        self.portfolio_entries_container = ctk.CTkScrollableFrame(portfolio, height=220)
        self.portfolio_entries_container.grid(row=4, column=0, padx=20, pady=4, sticky="nsew")
        self.portfolio_entries_container.grid_columnconfigure(1, weight=1)

        ctk.CTkButton(portfolio, text="Calcular carteira", command=self.run_portfolio_projection).grid(
            row=5, column=0, padx=20, pady=8, sticky="ew"
        )
        portfolio_table_frame = ctk.CTkFrame(portfolio, fg_color="transparent")
        portfolio_table_frame.grid(row=6, column=0, padx=20, pady=(0, 20), sticky="nsew")
        portfolio_table_frame.columnconfigure(0, weight=1)
        portfolio_table_frame.rowconfigure(0, weight=1)
        port_columns = ("month", "cotas", "renda", "acumulada")
        self.portfolio_tree = ttk.Treeview(
            portfolio_table_frame,
            columns=port_columns,
            show="headings",
            height=10,
        )
        port_headings = {
            "month": ("Mes", 90),
            "cotas": ("Cotas totais", 120),
            "renda": ("Renda/m consolidada", 170),
            "acumulada": ("Renda acumulada", 150),
        }
        for key, (title, width) in port_headings.items():
            self.portfolio_tree.heading(key, text=title)
            self.portfolio_tree.column(key, width=width, anchor="center")
        self.portfolio_tree.tag_configure("evenrow", background=self._table_bg, foreground="#e2e8f0")
        self.portfolio_tree.tag_configure("oddrow", background=self._table_alt_bg, foreground="#e2e8f0")
        port_scroll = ttk.Scrollbar(portfolio_table_frame, orient="vertical", command=self.portfolio_tree.yview)
        self.portfolio_tree.configure(yscrollcommand=port_scroll.set)
        self.portfolio_tree.grid(row=0, column=0, sticky="nsew")
        port_scroll.grid(row=0, column=1, sticky="ns")

    # Data refresh helpers -----------------------------------------------------
    def refresh_all(self, select: Optional[str] = None) -> None:
        self.refresh_table(select)
        self.update_dashboard()
        self.update_projection_controls()

    def refresh_table(self, select: Optional[str] = None) -> None:
        for item in self.tree.get_children():
            self.tree.delete(item)
        fiis = self.tracker.list_fiis()
        for idx, fii in enumerate(fiis):
            row = (
                fii.ticker,
                fii.name,
                fii.sector,
                f"{fii.total_cotas():.2f}",
                f"R$ {fii.total_invested():.2f}",
                f"R$ {fii.average_price():.2f}",
                f"R$ {fii.average_dividend_per_cota():.2f}",
                f"R$ {(fii.total_cotas() * fii.average_dividend_per_cota()):.2f}",
            )
            tag = "oddrow" if idx % 2 else "evenrow"
            self.tree.insert("", "end", iid=fii.ticker, text=fii.ticker, values=row, tags=(tag,))
        target = select or (fiis[0].ticker if fiis else None)
        if target and target in self.tree.get_children(""):
            self.tree.selection_set(target)
            self.show_details(target)
        else:
            self.active_ticker = None
            self._update_details_text("Nenhum FII cadastrado. Clique em 'Adicionar FII'.")

    def update_projection_controls(self) -> None:
        fiis = self.tracker.list_fiis()
        values = [fii.ticker for fii in fiis] or ["-"]
        self.proj_fii_option.configure(values=values)
        if fiis:
            if self.proj_fii_var.get() not in values:
                self.proj_fii_var.set(values[0])
        else:
            self.proj_fii_var.set("-")

        for widget in self.portfolio_entries_container.winfo_children():
            widget.destroy()
        self.portfolio_entries.clear()

        if not fiis:
            ctk.CTkLabel(
                self.portfolio_entries_container,
                text="Nenhum FII cadastrado.",
            ).grid(row=0, column=0, padx=10, pady=10, sticky="w")
            return

        for idx, fii in enumerate(fiis):
            row = ctk.CTkFrame(self.portfolio_entries_container, fg_color="transparent")
            row.grid(row=idx, column=0, padx=5, pady=4, sticky="ew")
            row.grid_columnconfigure(1, weight=1)
            ctk.CTkLabel(row, text=fii.ticker, width=80).grid(row=0, column=0, padx=(5, 10), sticky="w")
            entry = ctk.CTkEntry(row, placeholder_text="0")
            entry.insert(0, "0")
            entry.grid(row=0, column=1, sticky="ew")
            self.portfolio_entries[fii.ticker] = entry

    # Detail helpers -----------------------------------------------------------
    def show_details(self, ticker: str) -> None:
        fii = self.tracker.find_fii(ticker)
        if not fii:
            return
        last = fii.last_record()
        lines = [
            f"{fii.ticker} - {fii.name}",
            f"Setor: {fii.sector or 'n/d'}",
            f"Cotas: {fii.total_cotas():.2f}",
            f"Investido: R$ {fii.total_invested():.2f}",
            f"Preco medio: R$ {fii.average_price():.2f}",
            f"Dividendo medio/cota: R$ {fii.average_dividend_per_cota():.2f}",
            f"Renda projetada: R$ {(fii.total_cotas() * fii.average_dividend_per_cota()):.2f}/mes",
        ]
        lines.append(f"Total recebido: R$ {fii.total_dividends_received():.2f}")
        if last:
            lines.append(f"Ultimo mes: {last.month} - Total R$ {(last.dividend_total or 0):.2f}")
            if last.notes:
                lines.append(f"Obs: {last.notes}")
        self.active_ticker = ticker
        self._update_details_text("\n".join(lines))

    def _update_details_text(self, text: str) -> None:
        self.details.configure(state="normal")
        self.details.delete("1.0", "end")
        self.details.insert("end", text)
        self.details.configure(state="disabled")

    def on_tree_select(self, event: object) -> None:
        selected = self.tree.selection()
        if selected:
            self.show_details(selected[0])

    # Projection actions -------------------------------------------------------
    def run_individual_projection(self) -> None:
        ticker = self.proj_fii_var.get()
        if not ticker or ticker == "-":
            messagebox.showinfo("Selecione", "Cadastre e selecione um FII.")
            return
        months = int(self.proj_months_entry.get() or "12")
        monthly_cotas = parse_float(self.proj_cotas_entry.get() or "1", 1.0)
        window_value = self.proj_window_entry.get().strip()
        window = int(window_value) if window_value else None
        try:
            points = self.tracker.project_income(ticker, months=months, monthly_cotas=monthly_cotas, window=window)
        except ValueError as err:
            messagebox.showerror("Erro", str(err))
            return
        for item in self.proj_tree.get_children():
            self.proj_tree.delete(item)
        if not points:
            messagebox.showinfo("Sem dados", "Sem dados de dividendos para projetar.")
            return
        for idx, point in enumerate(points):
            tag = "oddrow" if idx % 2 else "evenrow"
            self.proj_tree.insert(
                "",
                "end",
                values=(
                    point.month,
                    f"{point.projected_cotas:.2f}",
                    f"R$ {point.projected_income:.2f}",
                    f"R$ {point.cumulative_income:.2f}",
                    f"{point.reinvested_cotas:.2f}",
                    f"{point.combined_cotas:.2f}",
                    f"R$ {point.combined_income:.2f}",
                ),
                tags=(tag,),
            )

    def run_portfolio_projection(self) -> None:
        fiis = self.tracker.list_fiis()
        if not fiis:
            messagebox.showinfo("Cadastre um FII", "Cadastre FIIs para projetar.")
            return
        months = int(self.port_months_entry.get() or "12")
        window_value = self.port_window_entry.get().strip()
        window = int(window_value) if window_value else None
        plan = {
            ticker: parse_float(entry.get() or "0", 0.0)
            for ticker, entry in self.portfolio_entries.items()
        }
        points = self.tracker.project_portfolio(months=months, monthly_plan=plan, window=window)
        for item in self.portfolio_tree.get_children():
            self.portfolio_tree.delete(item)
        if not points:
            messagebox.showinfo("Sem dados", "Sem dados de dividendos para projetar.")
            return
        for idx, point in enumerate(points):
            tag = "oddrow" if idx % 2 else "evenrow"
            self.portfolio_tree.insert(
                "",
                "end",
                values=(
                    point.month,
                    f"{point.projected_cotas:.2f}",
                    f"R$ {point.projected_income:.2f}",
                    f"R$ {point.cumulative_income:.2f}",
                ),
                tags=(tag,),
            )

    # Buttons ------------------------------------------------------------------
    def reload_data(self) -> None:
        self.tracker.refresh()
        self.refresh_all(select=self.active_ticker)
        self.status.configure(text=f"Atualizado. Arquivo: {self.tracker.data_path}")

    def open_add_modal(self) -> None:
        FiiModal(self, self.tracker, self._after_change, self.active_ticker)

    def open_month_modal(self) -> None:
        if not self.active_ticker:
            messagebox.showinfo("Selecione", "Escolha um FII primeiro.")
            return
        MonthModal(self, self.tracker, self.active_ticker, self._after_change)

    def open_history_modal(self) -> None:
        if not self.active_ticker:
            messagebox.showinfo("Selecione", "Escolha um FII primeiro.")
            return
        HistoryModal(self, self.tracker, self.active_ticker)

    def open_edit_history_modal(self) -> None:
        if not self.active_ticker:
            messagebox.showinfo("Selecione", "Escolha um FII primeiro.")
            return
        EditHistoryModal(self, self.tracker, self.active_ticker, self._after_change)

    def open_projection_modal(self) -> None:
        if not self.active_ticker:
            messagebox.showinfo("Selecione", "Escolha um FII primeiro.")
            return
        ProjectionModal(self, self.tracker, self.active_ticker)

    def _after_change(self, ticker: Optional[str]) -> None:
        self.tracker.refresh()
        self.refresh_all(select=ticker or self.active_ticker)
        self.status.configure(text="Dados salvos.")


def run(data_path: Optional[str] = None) -> None:
    app = TrackerWindow(data_path=data_path)
    app.mainloop()


if __name__ == "__main__":
    run()
