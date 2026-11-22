//! The help texts of the Settings page: what a setting is for and the background behind it. A row of the page shows its "?" button
//! when its name (English or German, as the row shows it) has an entry here; the button opens the text in a window.

use crate::i18n::Lang;

/// The help of one setting: its name as the rows show it in English and German, and the text in both languages.
pub struct HelpEntry {
    /// The name of the row in English.
    pub en: &'static str,
    /// The name of the row in German.
    pub de: &'static str,
    /// The help text in English.
    pub text_en: &'static str,
    /// The help text in German.
    pub text_de: &'static str,
}

impl HelpEntry {
    /// The name in `lang`.
    #[must_use]
    pub fn title(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => self.en,
            Lang::De => self.de,
        }
    }

    /// The text in `lang`.
    #[must_use]
    pub fn text(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => self.text_en,
            Lang::De => self.text_de,
        }
    }
}

/// The help of the row named `name` (in either language), if there is one.
#[must_use]
pub fn help_for(name: &str) -> Option<&'static HelpEntry> {
    HELP.iter().find(|entry| entry.en == name || entry.de == name)
}

/// Every help text of the Settings page.
pub static HELP: &[HelpEntry] = &[
    // General / Broker
    HelpEntry {
        en: "IBKR TWS Host",
        de: "IBKR TWS Host",
        text_en: "The computer TWS (or IB Gateway) runs on. 127.0.0.1 means this computer. Another computer in your network is entered by its IP address; TWS must then allow API connections from it (TWS: Global Configuration > API > Settings > Trusted IPs).",
        text_de: "Der Rechner, auf dem die TWS (oder das IB Gateway) läuft. 127.0.0.1 steht für diesen Rechner. Läuft die TWS auf einem anderen Rechner im Netzwerk, trägst du dessen IP-Adresse ein. Die TWS muss API-Verbindungen von dort dann ausdrücklich zulassen (in der TWS unter Global Configuration > API > Settings > Trusted IPs).",
    },
    HelpEntry {
        en: "IBKR TWS Port",
        de: "IBKR TWS Port",
        text_en: "The socket port TWS listens on for the API. The defaults are 7497 for a paper account and 7496 for a live account in TWS, 4002 and 4001 in IB Gateway. It must be the port set in TWS under Global Configuration > API > Settings.",
        text_de: "Über diesen Port nimmt die TWS API-Verbindungen an. Üblich sind 7497 für ein Paper-Konto und 7496 für ein Live-Konto, beim IB Gateway 4002 und 4001. Der Wert muss mit dem Port übereinstimmen, der in der TWS unter Global Configuration > API > Settings eingestellt ist.",
    },
    HelpEntry {
        en: "IBKR Client ID",
        de: "IBKR Client-ID",
        text_en: "Every program connected to TWS at the same time needs a client id of its own; a second connection with the same id is refused. TWS keeps the orders apart by it: on a connect TradePilot only sees the working orders placed under its own client id. So keep the id your open trades were placed under, or their stops and targets are reported as missing. Empty means 1.",
        text_de: "Jedes Programm, das gleichzeitig mit der TWS verbunden ist, braucht eine eigene Client-ID. Eine zweite Verbindung mit derselben ID lehnt die TWS ab. Außerdem ordnet die TWS die Orders anhand dieser ID zu: Nach dem Verbinden sieht TradePilot nur die offenen Orders, die unter seiner eigenen ID aufgegeben wurden. Ändere die ID deshalb nicht, solange Trades offen sind, sonst meldet TradePilot deren Stops und Ziele als fehlend. Leer bedeutet 1.",
    },
    HelpEntry {
        en: "Maintain Connection",
        de: "Verbindung aufrechterhalten",
        text_en: "Reconnects to TWS by itself when the connection is lost (TWS restarted, network gone). Always: at any time. Market Hours: only while the US market is open, so a TWS shut down at night stays alone. Disabled: only by hand. Stops and targets keep working in TWS while TradePilot is not connected; what TradePilot does itself (adjustments, timed exits, schedules) waits for the connection.",
        text_de: "Baut die Verbindung zur TWS automatisch wieder auf, wenn sie abbricht, etwa nach einem Neustart der TWS oder bei Netzwerkproblemen. „Immer“ versucht es rund um die Uhr. „Während der Handelszeit“ nur, solange der US-Markt geöffnet ist; wenn du die TWS nachts beendest, bleibt sie also aus. „Deaktiviert“ verbindet nur auf Knopfdruck. Stops und Ziele liegen bei der TWS und bleiben auch ohne Verbindung aktiv. Alles, was TradePilot selbst erledigt (Stop-Anpassungen, Zeit-Exits, Zeitpläne), ruht dagegen, bis die Verbindung wieder steht.",
    },
    HelpEntry {
        en: "Convert 0 DTE Stops to Short",
        de: "0-DTE-Stops in Short umwandeln",
        text_en: "On the day a trade expires, from this New York time on, its stop buys back only the shorts and no longer sells the longs. Late in the last day a long that is out of the money is worth next to nothing, and a combo order that has to sell it fills worse or not at all; buying back the short alone is the quicker, surer exit. The stop keeps its price. Credit trades with a stop of their own only.",
        text_de: "Am Verfallstag kauft der Stop eines Trades ab dieser Uhrzeit (New Yorker Zeit) nur noch die Shorts zurück und verkauft die Longs nicht mehr mit. Kurz vor dem Verfall ist ein Long aus dem Geld kaum noch etwas wert, und eine Kombi-Order, die ihn mitverkaufen muss, wird schlechter oder gar nicht ausgeführt. Nur den Short zurückzukaufen ist dann der schnellere und sicherere Ausstieg. Der Stop-Preis bleibt gleich. Gilt nur für Credit-Trades mit eigenem Stop.",
    },
    HelpEntry {
        en: "Market Close",
        de: "Marktschluss",
        text_en: "The time the market closes, New York time (normally 4:00 PM). Templates written for another close (an exchange that closes at another time, say) have their exit times shifted by the difference when they are imported. The trading session itself follows the exchange calendar, early closes included.",
        text_de: "Die Uhrzeit des Handelsschlusses in New Yorker Zeit, normalerweise 16:00 Uhr. Wird ein Template importiert, das für eine andere Schlusszeit erstellt wurde, verschiebt TradePilot dessen Exit-Zeiten um den Unterschied. Die Handelszeiten selbst richten sich nach dem Börsenkalender, verkürzte Handelstage eingeschlossen.",
    },
    HelpEntry {
        en: "Account numbers to ignore",
        de: "Zu ignorierende Kontonummern",
        text_en: "Accounts of your login that TradePilot is not to offer or use, separated by commas: a family member's account, an IRA you do not trade options in. They are left out of the account list of Instant Order and of the schedules.",
        text_de: "Konten deines Logins, die TradePilot weder anzeigen noch verwenden soll, durch Kommas getrennt, zum Beispiel das Konto eines Familienmitglieds oder ein Altersvorsorgekonto ohne Optionshandel. Sie erscheinen dann nicht in der Kontoauswahl der Sofortorder und der Zeitpläne.",
    },
    HelpEntry {
        en: "Logging Level",
        de: "Protokollierungsstufe",
        text_en: "Info writes what TradePilot decides and sends. Debug adds every order exactly as it went to TWS and what TWS answered: useful to find out why an order did not do what you expected, but the log grows quickly. Switch it back to Info afterwards.",
        text_de: "„Info“ protokolliert, was TradePilot entscheidet und an die TWS sendet. „Debug“ hält zusätzlich jede Order genau so fest, wie sie an die TWS ging, und dazu jede Antwort der TWS. Das hilft bei der Suche, warum eine Order sich anders verhalten hat als erwartet, lässt das Protokoll aber schnell wachsen. Stell danach wieder auf „Info“ zurück.",
    },
    HelpEntry {
        en: "Clear Logs",
        de: "Protokolle löschen",
        text_en: "Empties the log file after asking. The log is the record of every order decision; keep it as long as a trade from that time may still need explaining.",
        text_de: "Leert die Protokolldatei nach einer Sicherheitsabfrage. Das Protokoll dokumentiert jede Order-Entscheidung. Lösche es erst, wenn du die Trades aus diesem Zeitraum nicht mehr nachvollziehen musst.",
    },
    HelpEntry {
        en: "Clean up old log files",
        de: "Alte Protokolldateien bereinigen",
        text_en: "Once a day, and at every start, the log lines older than the interval below are removed, and so are other files in the log folder last changed before then. Without it the log only grows.",
        text_de: "Einmal täglich und bei jedem Start entfernt TradePilot Protokolleinträge, die älter sind als das Intervall darunter. Andere Dateien im Protokollordner, die seitdem nicht mehr geändert wurden, werden ebenfalls gelöscht. Ohne Bereinigung wächst das Protokoll unbegrenzt.",
    },
    HelpEntry {
        en: "Cleanup Interval (days)",
        de: "Bereinigungsintervall (Tage)",
        text_en: "How many days of log lines are kept by the cleanup above. Anything that is not a whole number from 1 up counts as 90.",
        text_de: "Wie viele Tage die Bereinigung darüber im Protokoll aufbewahrt. Steht hier keine ganze Zahl ab 1, gelten 90 Tage.",
    },
    HelpEntry {
        en: "Grid Font Size",
        de: "Schriftgröße (Tabellen)",
        text_en: "The text size of every table; the rows grow with it. It applies at once, also before you save.",
        text_de: "Die Schriftgröße in allen Tabellen; die Zeilenhöhe passt sich an. Die Änderung wirkt sofort, schon vor dem Speichern.",
    },
    HelpEntry {
        en: "Mobile Friendly Grid",
        de: "Mobilfreundliche Tabellen",
        text_en: "Compact tables for a narrow window: tighter rows and spacing, and columns that would get too narrow to read are left out (from the second to last backwards; the first and the last column always stay). Nothing is cut off at the edge.",
        text_de: "Kompakte Tabellen für schmale Fenster: Zeilen und Abstände werden enger, und Spalten, die zu schmal zum Lesen würden, werden ausgeblendet, von der vorletzten an rückwärts. Die erste und die letzte Spalte bleiben immer sichtbar. So wird am Rand nichts abgeschnitten.",
    },
    HelpEntry {
        en: "UI mode",
        de: "Oberflächenmodus",
        text_en: "Light shows only the pages needed day to day in the sidebar; Advanced shows every page (conditions, triggers, indicators, logs and more). Nothing is switched off by Light: schedules, triggers and exits keep working, their pages are only hidden.",
        text_de: "„Light“ zeigt in der Seitenleiste nur die Seiten, die du im Alltag brauchst. „Advanced“ zeigt alle Seiten, also auch Bedingungen, Trigger, Indikatoren, Protokolle und mehr. Light schaltet keine Funktion ab: Zeitpläne, Trigger und Exits arbeiten weiter, nur ihre Seiten sind ausgeblendet.",
    },
    HelpEntry {
        en: "Language",
        de: "Sprache",
        text_en: "The language of the screens and of the news translation. The log stays in English, so that it reads the same for everyone who helps you with it.",
        text_de: "Die Sprache der Oberfläche und der Nachrichtenübersetzung. Das Protokoll bleibt auf Englisch, damit es jeder lesen kann, der dir bei einem Problem hilft.",
    },
    HelpEntry {
        en: "Contact address for the translation service (optional)",
        de: "Kontaktadresse für den Übersetzungsdienst (optional)",
        text_en: "News in another language are translated by the free service MyMemory, which allows a limited amount of text per day. With an e-mail address of its own that limit is much higher. Use an address made for this, not your private one: it is sent to the service with every translation.",
        text_de: "Nachrichten in anderen Sprachen übersetzt der kostenlose Dienst MyMemory, der pro Tag nur eine begrenzte Textmenge erlaubt. Mit einer hinterlegten E-Mail-Adresse liegt diese Grenze deutlich höher. Verwende dafür eine eigene Adresse und nicht deine private, denn sie wird bei jeder Übersetzung an den Dienst übermittelt.",
    },
    // Notifications
    HelpEntry {
        en: "Notification Email Address",
        de: "Benachrichtigungs-E-Mail-Adresse",
        text_en: "Where the notification mails go. Warnings (missing stop, trade beyond its stop, margin problems) are sent there too, whatever the switches below say. Several addresses are separated by commas.",
        text_de: "An diese Adresse gehen die Benachrichtigungen. Warnungen wie ein fehlender Stop, ein Trade jenseits seines Stops oder Margin-Probleme werden immer dorthin geschickt, unabhängig von den Schaltern darunter. Mehrere Adressen trennst du mit Kommas.",
    },
    HelpEntry {
        en: "Entry filled",
        de: "Trade eröffnet",
        text_en: "A message as soon as TWS reports an opening order of TradePilot completely filled, with the trade, the price and its stop.",
        text_de: "Eine Nachricht, sobald die TWS eine Eröffnungsorder von TradePilot als vollständig ausgeführt meldet, mit Trade, Preis und Stop.",
    },
    HelpEntry {
        en: "Stop filled / closed",
        de: "Trade gestoppt/geschlossen",
        text_en: "A message when a trade ends by its stop, its profit target or a close order, with the closing price and the profit or loss.",
        text_de: "Eine Nachricht, wenn ein Trade über seinen Stop, sein Gewinnziel oder eine Schließorder beendet wird, mit Schließpreis und Gewinn oder Verlust.",
    },
    HelpEntry {
        en: "Host",
        de: "Host",
        text_en: "The SMTP server of your mail provider that sends the mails, e.g. smtp.gmail.com. Your provider's help names it.",
        text_de: "Der Postausgangsserver (SMTP) deines E-Mail-Anbieters, zum Beispiel smtp.gmail.com. Du findest ihn in der Hilfe deines Anbieters.",
    },
    HelpEntry {
        en: "Port",
        de: "Port",
        text_en: "The port of the SMTP server: usually 587 with STARTTLS, 465 with a connection encrypted from the start.",
        text_de: "Der Port des SMTP-Servers, meist 587 mit STARTTLS oder 465 bei einer von Anfang an verschlüsselten Verbindung.",
    },
    HelpEntry {
        en: "Username",
        de: "Benutzername",
        text_en: "The login of the mail account that sends, usually its full address.",
        text_de: "Der Anmeldename des Absenderkontos, meist die vollständige E-Mail-Adresse.",
    },
    HelpEntry {
        en: "Password",
        de: "Passwort",
        text_en: "The password of the sending account. Many providers want an app password made for this instead of the account password (Gmail: Google account > Security > App passwords). It is stored in the settings file of this computer only.",
        text_de: "Das Passwort des Absenderkontos. Viele Anbieter verlangen statt des normalen Kontopassworts ein eigens erzeugtes App-Passwort (bei Gmail: Google-Konto > Sicherheit > App-Passwörter). Es wird nur in der Einstellungsdatei auf diesem Rechner gespeichert.",
    },
    HelpEntry {
        en: "STARTTLS",
        de: "STARTTLS",
        text_en: "Encrypts the connection to the mail server after it is opened (port 587). Leave it on unless your server says otherwise; without encryption the password goes over the network in the clear.",
        text_de: "Verschlüsselt die Verbindung zum Mailserver nach dem Verbindungsaufbau (Port 587). Lass die Option eingeschaltet, sofern dein Anbieter nichts anderes vorgibt, denn ohne Verschlüsselung wird das Passwort im Klartext übertragen.",
    },
    HelpEntry {
        en: "Notification Webhook URL",
        de: "Benachrichtigungs-Webhook-URL",
        text_en: "An address the notifications are also posted to as JSON: a chat bot (Discord, Slack, Telegram through a bridge) or your own service. Empty: no webhook.",
        text_de: "Eine Adresse, an die TradePilot die Benachrichtigungen zusätzlich als JSON schickt, etwa an einen Chat-Bot (Discord, Slack, Telegram über eine Brücke) oder an einen eigenen Dienst. Leer bedeutet: kein Webhook.",
    },
    HelpEntry {
        en: "Webhook triggers",
        de: "Webhook-Trigger",
        text_en: "Starts the local receiver for webhook signals (from a charting service through a tunnel, say) that enter trades by their triggers. It listens on this computer only; reaching it from outside is up to a tunnel you set up.",
        text_de: "Startet den lokalen Empfänger für Webhook-Signale, die über ihre Trigger Trades eröffnen, etwa Signale eines Chart-Dienstes über einen Tunnel. Der Empfänger ist nur auf diesem Rechner erreichbar. Für den Zugriff von außen musst du selbst einen Tunnel einrichten.",
    },
    HelpEntry {
        en: "Test and token",
        de: "Test und Token",
        text_en: "Sends a test signal to the receiver and shows the token a signal has to carry. A signal without the right token, or older than ten minutes, is refused.",
        text_de: "Schickt ein Testsignal an den Empfänger und zeigt das Token, das jedes Signal enthalten muss. Signale ohne gültiges Token oder älter als zehn Minuten werden abgewiesen.",
    },
    // Trading Features
    HelpEntry {
        en: "Default for new templates",
        de: "Vorgabe für neue Templates",
        text_en: "What happens to a long that a Short stop left behind (the stop bought back only the short). Ignore: it stays in the account; a later trade can use it as its long, and it may still gain if the market turns. Sell: it is sold at market at once, if it has a bid. Each template can set it for itself.",
        text_de: "Was mit einem Long passiert, den ein Short-Stop zurückgelassen hat, weil der Stop nur den Short zurückgekauft hat. „Ignore“: Der Long bleibt im Konto. Ein späterer Trade kann ihn als Long verwenden, und wenn der Markt dreht, kann er noch an Wert gewinnen. „Sell“: Der Long wird sofort zum Marktpreis verkauft, sofern es einen Bid gibt. Jedes Template kann das auch selbst festlegen.",
    },
    HelpEntry {
        en: "Split short stops on IF and IC-Full",
        de: "Short-Stops bei IF und IC-Full aufteilen",
        text_en: "An IronFly, IronCondor-Full or ShortStrangle is one trade with one stop that buys back both shorts together. Once one short is nearly worthless, that common stop protects the other side badly: its price was meant for both. With this switch the trade is then split into one trade per side, each with its own stop on its short and its own target, like the two sides of an IronCondor. First the stop and the target are cancelled, then the sides are made, priced from the fills of their legs, then their orders are placed. Only for a Short stop type.",
        text_de: "Ein IronFly, IronCondor-Full oder ShortStrangle ist ein einziger Trade mit einem gemeinsamen Stop, der beide Shorts zusammen zurückkauft. Ist einer der Shorts fast wertlos, schützt dieser gemeinsame Stop die andere Seite nur noch schlecht, denn sein Preis war für beide Seiten berechnet. Mit diesem Schalter wird der Trade dann in zwei Trades aufgeteilt, einen pro Seite, jeder mit eigenem Stop auf seinem Short und eigenem Gewinnziel, so wie die beiden Seiten eines IronCondor. Dazu werden zuerst Stop und Ziel storniert, dann die beiden Seiten mit den Ausführungspreisen ihrer Beine angelegt und anschließend ihre Orders aufgegeben. Funktioniert nur mit dem Stop-Typ Short.",
    },
    HelpEntry {
        en: "Bid price to trigger split",
        de: "Bid-Preis zum Auslösen des Splits",
        text_en: "The split above starts when a short's bid is at this price or below (a short without any bid counts too) while it still has an ask. 0.05 means: as good as worthless.",
        text_de: "Die Aufteilung darüber beginnt, sobald der Bid eines Shorts auf diesen Preis oder darunter fällt, solange der Short noch einen Ask hat. Ein Short ganz ohne Bid zählt ebenfalls. 0,05 heißt: so gut wie wertlos.",
    },
    HelpEntry {
        en: "Convert Short Stop to Vertical",
        de: "Short-Stop in Vertical umwandeln",
        text_en: "A template with the stop type Vertical wants its stop to sell the long with the short. A long without a bid at the fill (or taken out by the conversion below) is left out of the stop. With this switch it is put back once its bid is 0.10 or more again (and above the conversion price below), so that a stop sells what the long is still worth. Every day only before the 0-DTE time.",
        text_de: "Bei einem Template mit Stop-Typ Vertical soll der Stop den Long zusammen mit dem Short verkaufen. Hatte der Long bei der Ausführung keinen Bid oder hat ihn die Umwandlung darunter entfernt, ist er nicht im Stop enthalten. Mit diesem Schalter wird er wieder aufgenommen, sobald sein Bid mindestens 0,10 beträgt und über dem Umwandlungspreis darunter liegt. So verkauft der Stop mit, was der Long noch wert ist. Das geschieht täglich nur vor der 0-DTE-Uhrzeit.",
    },
    HelpEntry {
        en: "Convert Vertical Stop to Short",
        de: "Vertical-Stop in Short umwandeln",
        text_en: "A long the stop has to sell whose bid has fallen to the price below (or that has no bid at all) is taken out of the stop: it brings next to nothing, and a combo order that has to sell it can fill late or badly just when it has to be quick. The stop then buys back the short alone, at the same price.",
        text_de: "Fällt der Bid eines Longs, den der Stop verkaufen müsste, auf den Preis darunter (oder hat er gar keinen Bid mehr), nimmt TradePilot ihn aus dem Stop. Er bringt ohnehin kaum noch etwas, und eine Kombi-Order, die ihn mitverkaufen muss, wird womöglich gerade dann spät oder schlecht ausgeführt, wenn es schnell gehen muss. Der Stop kauft danach nur noch den Short zurück, zum selben Preis.",
    },
    HelpEntry {
        en: "Convert Vertical to Short bid price",
        de: "Bid-Preis für Vertical-zu-Short",
        text_en: "The bid of the long at or below which the conversion above takes it out of the stop, e.g. 0.05.",
        text_de: "Ab diesem Bid des Longs (oder darunter) nimmt die Umwandlung darüber ihn aus dem Stop, zum Beispiel 0,05.",
    },
    HelpEntry {
        en: "Target Profit",
        de: "Zielgewinn",
        text_en: "When today's profit (open trades at their current value plus closed ones) stays above this amount for the delay, the entries still waiting today are cancelled and you get a message: a good day is not given back by new trades. The open trades are not closed; they keep their stops and targets. Set it to 0 to switch it off.",
        text_de: "Liegt der heutige Gewinn (offene Trades zum aktuellen Wert plus geschlossene) für die eingestellte Verzögerung über diesem Betrag, werden alle heute noch ausstehenden Einstiege storniert, und du erhältst eine Nachricht. So verspielst du einen guten Tag nicht mit neuen Trades. Offene Trades werden dabei nicht geschlossen, sie behalten ihre Stops und Ziele. 0 schaltet die Funktion aus.",
    },
    HelpEntry {
        en: "Delay Seconds",
        de: "Verzögerung (Sek.)",
        text_en: "How long the profit must stay above the target before it counts, so that a single quote spike does not end the day. Empty = 15 seconds.",
        text_de: "So lange muss der Gewinn über dem Ziel bleiben, bevor es als erreicht gilt. Eine einzelne Kursspitze beendet den Handelstag so nicht. Leer bedeutet 15 Sekunden.",
    },
    HelpEntry {
        en: "Amount",
        de: "Betrag",
        text_en: "Templates can size a trade as a percent of the net liquidity of the account (PctOfNetLiq). With an amount here that amount is used instead of what TWS reports: to size trades for only a part of a bigger account, or to keep the size steady while the account value moves.",
        text_de: "Templates können die Positionsgröße als Prozentsatz der Net Liquidity des Kontos festlegen (PctOfNetLiq). Steht hier ein Betrag, rechnet TradePilot mit diesem statt mit dem von der TWS gemeldeten Wert. Das ist nützlich, wenn du nur mit einem Teil eines größeren Kontos handeln oder die Positionsgröße unabhängig von Schwankungen des Kontowerts halten möchtest.",
    },
    // Market Data
    HelpEntry {
        en: "Market Data Provider",
        de: "Marktdaten-Anbieter",
        text_en: "Where the prices come from. TradePilot takes them from TWS only; that needs the market data subscriptions for the symbols at Interactive Brokers (for SPX options: OPRA and the CBOE indices).",
        text_de: "Die Quelle der Kurse. TradePilot bezieht sie ausschließlich von der TWS. Dafür brauchst du bei Interactive Brokers die passenden Marktdaten-Abonnements, für SPX-Optionen etwa OPRA und die CBOE-Indizes.",
    },
    HelpEntry {
        en: "Estimate SPX Extended Hours",
        de: "SPX außerbörsliche Zeiten schätzen",
        text_en: "SPX is an index: outside the regular session TWS sends no price for it. SPY follows it closely and trades before the open and after the close. With this switch the top bar shows SPX estimated as the SPY price times the ratio of the two (taken while both are live, else from the last closes), marked as an estimate. It is shown only and never used for an order or a condition.",
        text_de: "SPX ist ein Index, für den die TWS außerhalb der regulären Handelszeit keinen Kurs liefert. SPY bildet SPX sehr genau nach und wird auch vor- und nachbörslich gehandelt. Mit diesem Schalter zeigt die Kopfleiste einen geschätzten SPX-Wert: den SPY-Kurs mal dem Verhältnis beider Werte, ermittelt während der Handelszeit oder sonst aus den letzten Schlusskursen. Die Schätzung ist gekennzeichnet, dient nur zur Anzeige und wird nie für Orders oder Bedingungen verwendet.",
    },
    HelpEntry {
        en: "Price Symbol",
        de: "Preis-Symbol",
        text_en: "The symbol whose price the top bar shows. Without a choice it is SPX. Only configured symbols are offered, because only their prices are subscribed.",
        text_de: "Das Symbol, dessen Kurs in der Kopfleiste steht. Ohne Auswahl ist es SPX. Zur Wahl stehen nur konfigurierte Symbole, denn nur für sie sind Kurse abonniert.",
    },
    HelpEntry {
        en: "Delta Symbol",
        de: "Delta-Symbol",
        text_en: "The underlying whose total delta of today's trades the Dashboard shows: how much your open positions gain or lose when it moves by one point.",
        text_de: "Der Basiswert, für den das Dashboard das Gesamt-Delta der heutigen Trades anzeigt. Es gibt an, wie viel deine offenen Positionen gewinnen oder verlieren, wenn sich der Basiswert um einen Punkt bewegt.",
    },
    HelpEntry {
        en: "Symbol",
        de: "Symbol",
        text_en: "An underlying whose options TradePilot is to load (SPX, XSP, QQQ ...). Only the options of configured symbols get live quotes, and templates can only trade those.",
        text_de: "Ein Basiswert, dessen Optionen TradePilot laden soll, etwa SPX, XSP oder QQQ. Nur für die Optionen konfigurierter Symbole gibt es Live-Kurse, und nur mit ihnen können Templates handeln.",
    },
    HelpEntry {
        en: "Required DTEs",
        de: "Benötigte DTEs",
        text_en: "Which expirations to load, as days to expiration separated by commas: 0 = today, 1 = the next trading day, \"0,1,7\". Every expiration costs market data lines, so load only those your templates trade.",
        text_de: "Welche Verfallstermine geladen werden, angegeben als Tage bis zum Verfall und durch Kommas getrennt: 0 ist heute, 1 der nächste Handelstag, zum Beispiel „0,1,7“. Jeder Verfallstermin belegt Marktdatenzeilen, lade also nur die, die deine Templates tatsächlich handeln.",
    },
    HelpEntry {
        en: "Put OTM (points)",
        de: "Put OTM (Punkte)",
        text_en: "How far below the underlying price puts get live quotes. Interactive Brokers allows only a limited number of market data lines at the same time (often 100); this window decides which options use them. It must reach the strikes your templates pick. Empty = the default of the symbol.",
        text_de: "Bis zu wie viele Punkte unter dem Kurs des Basiswerts Puts mit Live-Kursen versorgt werden. Interactive Brokers erlaubt nur eine begrenzte Zahl gleichzeitiger Marktdatenzeilen, oft 100. Dieser Bereich legt fest, welche Optionen sie bekommen, und muss die Strikes abdecken, die deine Templates auswählen. Leer bedeutet: Vorgabe des Symbols.",
    },
    HelpEntry {
        en: "Call OTM (points)",
        de: "Call OTM (Punkte)",
        text_en: "How far above the underlying price calls get live quotes (see Put OTM). Empty = the default of the symbol.",
        text_de: "Bis zu wie viele Punkte über dem Kurs des Basiswerts Calls mit Live-Kursen versorgt werden (siehe Put OTM). Leer bedeutet: Vorgabe des Symbols.",
    },
    HelpEntry {
        en: "Put ITM (points)",
        de: "Put ITM (Punkte)",
        text_en: "How far above the price, into the money, puts still get quotes: for a trade whose short has gone in the money, and for strike choices close to the price. Empty = the default of the symbol.",
        text_de: "Bis zu wie viele Punkte über dem Kurs, also im Geld, Puts noch Kurse erhalten. Das braucht es für Trades, deren Short ins Geld gelaufen ist, und für Strikes nahe am aktuellen Kurs. Leer bedeutet: Vorgabe des Symbols.",
    },
    HelpEntry {
        en: "Call ITM (points)",
        de: "Call ITM (Punkte)",
        text_en: "How far below the price, into the money, calls still get quotes (see Put ITM). Empty = the default of the symbol.",
        text_de: "Bis zu wie viele Punkte unter dem Kurs, also im Geld, Calls noch Kurse erhalten (siehe Put ITM). Leer bedeutet: Vorgabe des Symbols.",
    },
    HelpEntry {
        en: "Extended Quote Strikes",
        de: "Erweiterte Quote-Strikes",
        text_en: "Quotes more strikes than the window asks for, for templates that pick far out of the money. It uses more market data lines.",
        text_de: "Lädt Kurse für mehr Strikes, als der eingestellte Bereich vorsieht, für Templates, die weit aus dem Geld auswählen. Das belegt zusätzliche Marktdatenzeilen.",
    },
    // Safety Features
    HelpEntry {
        en: "Close trade on stop margin rejection",
        de: "Trade bei Margin-Ablehnung des Stops schließen",
        text_en: "TWS can refuse a stop for margin, e.g. when the account is near its limit. The trade is then open without protection. With this switch it is closed at market at once instead. Only for a real rejection (TWS error 201), not for a margin warning that leaves the stop working.",
        text_de: "Die TWS kann einen Stop wegen unzureichender Margin ablehnen, etwa wenn das Konto an seiner Grenze ist. Der Trade wäre dann ungeschützt. Mit diesem Schalter schließt TradePilot ihn in diesem Fall sofort zum Marktpreis. Das gilt nur für eine echte Ablehnung (TWS-Fehler 201), nicht für eine bloße Margin-Warnung, bei der der Stop aktiv bleibt.",
    },
    HelpEntry {
        en: "Close trade beyond stop price",
        de: "Trade jenseits des Stop-Preises schließen",
        text_en: "A stop in TWS can fail to fire: a combo stop that TWS does not trigger, a fast market, a stop that was rejected. If closing the trade costs at least the percent below more than its stop, for longer than the wait below, TradePilot closes it itself. The warning \"Trade Beyond Stop\" is sent after 60 seconds either way.",
        text_de: "Ein Stop in der TWS löst nicht immer aus, etwa bei einem Kombi-Stop, den die TWS nicht auslöst, bei sehr schnellen Märkten oder wenn der Stop abgelehnt wurde. Liegt der Preis zum Schließen länger als die eingestellte Wartezeit um mindestens den eingestellten Prozentsatz über dem Stop, schließt TradePilot den Trade selbst. Die Warnung „Trade Beyond Stop“ kommt in jedem Fall nach 60 Sekunden.",
    },
    HelpEntry {
        en: "% Beyond Stop",
        de: "% jenseits des Stops",
        text_en: "How far above the stop the cost to close must be for the close above, in percent of the stop: 10 with a stop at 2.00 means from 2.20. 0 = any cost above the stop.",
        text_de: "Um wie viel Prozent der Preis zum Schließen über dem Stop liegen muss, damit TradePilot schließt. 10 bei einem Stop von 2,00 heißt: ab 2,20. Bei 0 genügt jeder Preis über dem Stop.",
    },
    HelpEntry {
        en: "Wait (seconds)",
        de: "Warten (Sek.)",
        text_en: "How long the trade must stay beyond the stop before it is closed, so that a single wide quote does not close it. Empty = 60 seconds.",
        text_de: "So lange muss der Trade jenseits des Stops bleiben, bevor er geschlossen wird. Ein einzelner Ausreißer im Kurs löst so nichts aus. Leer bedeutet 60 Sekunden.",
    },
    HelpEntry {
        en: "Close Order Type",
        de: "Order-Typ zum Schließen",
        text_en: "Market fills at once at whatever the market gives. Limit asks the current cost to close and may not fill if the market runs on; it then stays working as a close order.",
        text_de: "„Market“ wird sofort zum aktuellen Marktpreis ausgeführt. „Limit“ bietet den aktuellen Preis zum Schließen und wird womöglich nicht ausgeführt, wenn der Markt weiterläuft. Die Order bleibt dann als offene Schließorder bestehen.",
    },
    HelpEntry {
        en: "Missing Stop Notifications",
        de: "Benachrichtigungen bei fehlendem Stop",
        text_en: "An urgent message when an open trade has no stop: the stop could not be placed, TWS cancelled or rejected it, or it is gone after a connect. A trade without a stop has no limit on its loss.",
        text_de: "Eine dringende Nachricht, wenn ein offener Trade keinen Stop hat: weil der Stop nicht platziert werden konnte, weil die TWS ihn storniert oder abgelehnt hat oder weil er nach dem Verbinden fehlt. Ohne Stop ist der Verlust eines Trades nicht begrenzt.",
    },
    HelpEntry {
        en: "High Slippage Notifications",
        de: "Benachrichtigungen bei hoher Slippage",
        text_en: "A message when a stop filled well beyond its price (see the percent below): a sign of a thin market or a stop type that does not fit the trade.",
        text_de: "Eine Nachricht, wenn ein Stop deutlich schlechter als zu seinem Stop-Preis ausgeführt wurde (siehe Prozentsatz darunter). Das deutet auf einen illiquiden Markt oder einen Stop-Typ hin, der nicht zum Trade passt.",
    },
    HelpEntry {
        en: "% Slippage to Notify",
        de: "% Slippage für Benachrichtigung",
        text_en: "From how many percent beyond the stop price a fill is reported: 20 with a stop at 2.00 means a fill at 2.40 or worse. 0 or empty = off.",
        text_de: "Ab wie viel Prozent Abweichung vom Stop-Preis eine Ausführung gemeldet wird. 20 bei einem Stop von 2,00 heißt: bei einer Ausführung zu 2,40 oder schlechter. 0 oder leer schaltet die Meldung aus.",
    },
    HelpEntry {
        en: "Retry trade entry on margin rejection",
        de: "Trade-Eintrag bei Margin-Ablehnung wiederholen",
        text_en: "An entry TWS refuses for margin is often possible a moment later, once another trade has closed or an order was cancelled. With this switch it is sent again up to 3 times, 20 seconds apart, under a new order id at the limit it started with; its schedule run keeps working meanwhile. Off: the entry is given up at once. An order held in TWS for review is never sent again.",
        text_de: "Ein Einstieg, den die TWS wegen unzureichender Margin ablehnt, klappt oft schon kurz darauf, etwa wenn ein anderer Trade geschlossen oder eine Order storniert wurde. Mit diesem Schalter sendet TradePilot den Einstieg bis zu dreimal im Abstand von 20 Sekunden erneut, jeweils mit neuer Order-ID und dem ursprünglichen Limit. Der zugehörige Zeitplan-Lauf bleibt so lange aktiv. Ist der Schalter aus, wird der Einstieg sofort aufgegeben. Orders, die in der TWS zur Prüfung zurückgehalten werden, sendet TradePilot nie erneut.",
    },
    HelpEntry {
        en: "Max. open contracts per symbol",
        de: "Max. offene Kontrakte je Symbol",
        text_en: "A new position is refused when the contracts (or combos) of its symbol that are open, together with the new ones, would be more than this. It keeps a mistake, a schedule that runs too often or a trigger that fires again and again from piling up a position far bigger than intended. Positions are counted from the open trades of TradePilot. 0 or empty = no limit.",
        text_de: "Eine neue Position wird abgelehnt, wenn die offenen Kontrakte (oder Kombis) ihres Symbols zusammen mit den neuen mehr als diese Zahl wären. So kann ein Fehler, ein zu oft laufender Zeitplan oder ein immer wieder auslösender Trigger keine Position aufbauen, die viel größer ist als gewollt. Gezählt werden die offenen Trades von TradePilot. 0 oder leer bedeutet: keine Grenze.",
    },
    HelpEntry {
        en: "Daily loss limit ($)",
        de: "Tagesverlust-Grenze ($)",
        text_en: "Once today's result (the closed trades plus the open ones at their current value) has lost this amount, no new position is opened for the rest of the New York day, even if the loss gets smaller again. Open trades are not closed by it: they keep their stops and targets. 0 or empty = no limit.",
        text_de: "Hat das heutige Ergebnis (geschlossene Trades plus offene zum aktuellen Wert) diesen Betrag verloren, eröffnet TradePilot für den Rest des New Yorker Handelstags keine neue Position mehr, auch wenn sich der Verlust wieder verringert. Offene Trades werden dadurch nicht geschlossen, sie behalten ihre Stops und Ziele. 0 oder leer bedeutet: keine Grenze.",
    },
    HelpEntry {
        en: "Max. new orders per minute",
        de: "Max. neue Orders pro Minute",
        text_en: "At most this many orders that open a position go out within one minute; one more is refused and the log says why. A safeguard against a fault that sends orders in a loop. Changing the price of a working order, stops, targets and close orders do not count. 0 or empty = no limit.",
        text_de: "Höchstens so viele Orders, die eine Position eröffnen, gehen innerhalb einer Minute hinaus; jede weitere wird abgelehnt, und das Log nennt den Grund. Das schützt vor einem Fehler, der Orders in einer Schleife sendet. Preisänderungen einer laufenden Order, Stops, Ziele und Schließorders zählen nicht mit. 0 oder leer bedeutet: keine Grenze.",
    },
    // Database Backup
    HelpEntry {
        en: "Daily Backups Enabled",
        de: "Tägliche Backups aktiviert",
        text_en: "Writes a backup of the settings once a day (at the first start of the day), unless this computer wrote one today already. The backups are in the backups folder of the data location and can be restored below. Trades and executions are in the database file; copy the data folder for a full backup.",
        text_de: "Sichert die Einstellungen einmal täglich beim ersten Start des Tages, sofern dieser Rechner heute noch keine Sicherung angelegt hat. Die Sicherungen liegen im Ordner „backups“ des Datenordners und lassen sich unten wiederherstellen. Trades und Ausführungen stehen in der Datenbankdatei; für eine vollständige Sicherung kopierst du den ganzen Datenordner.",
    },
    HelpEntry {
        en: "Computer Name",
        de: "Computername",
        text_en: "Goes into the file name of every backup, so that the backups of several computers in one shared folder stay apart.",
        text_de: "Wird in den Dateinamen jeder Sicherung übernommen. So lassen sich die Sicherungen mehrerer Rechner in einem gemeinsamen Ordner auseinanderhalten.",
    },
    HelpEntry {
        en: "Export the trades of the day after the market close",
        de: "Trades des Tages nach Handelsschluss exportieren",
        text_en: "After the close the trades of the day are written once to a CSV file in the exports folder, with the same columns as the export of the Reports page: for a spreadsheet, a journal or your tax records.",
        text_de: "Nach Handelsschluss schreibt TradePilot die Trades des Tages einmal in eine CSV-Datei im Ordner „exports“, mit denselben Spalten wie der Export auf der Berichtsseite. Die Datei eignet sich für eine Tabellenkalkulation, ein Trading-Journal oder die Steuerunterlagen.",
    },
    HelpEntry {
        en: "One-time instant backup.",
        de: "Einmaliges Sofort-Backup.",
        text_en: "Writes a backup of the settings now, e.g. before you change many of them.",
        text_de: "Sichert die Einstellungen sofort, zum Beispiel bevor du viele davon änderst.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_help_is_found_by_its_name_in_either_language() {
        assert_eq!(help_for("IBKR Client ID").map(|h| h.de), Some("IBKR Client-ID"));
        assert_eq!(help_for("IBKR Client-ID").map(|h| h.en), Some("IBKR Client ID"));
        assert!(help_for("no such setting").is_none());
    }

    /// The source of the rows of the Settings page (the webhook rows of the page are in the trigger module).
    const PAGE_SOURCES: [&str; 2] = [include_str!("app/settings_page.rs"), include_str!("app/triggers.rs")];

    /// The English names of the rows the page builds with `setting_row(ui, self.tr("..", ..)`.
    fn row_names() -> Vec<String> {
        let mut names = Vec::new();
        for source in PAGE_SOURCES {
            for (at, _) in source.match_indices("setting_row") {
                let rest = source.get(at..).unwrap_or_default();
                let Some(args) = rest.find('(').and_then(|open| rest.get(open + 1..)) else { continue };
                let args = args.trim_start().strip_prefix("ui,").unwrap_or(args).trim_start();
                if let Some(name) = args.strip_prefix("self.tr(\"").and_then(|text| text.split('"').next()) {
                    names.push(name.to_string());
                }
            }
        }
        names
    }

    #[test]
    fn every_row_of_the_settings_page_has_a_help_and_every_help_belongs_to_a_row() {
        let names = row_names();
        assert!(names.len() > 40, "the rows are found: {}", names.len());
        for name in &names {
            assert!(help_for(name).is_some(), "no help for the row \"{name}\"");
        }
        for entry in HELP {
            let quoted = format!("\"{}\"", entry.en);
            assert!(PAGE_SOURCES.iter().any(|source| source.contains(&quoted)), "the help \"{}\" belongs to no row", entry.en);
        }
    }

    #[test]
    fn every_help_has_a_name_and_a_text_in_both_languages_and_no_name_twice() {
        for entry in HELP {
            assert!(!entry.en.is_empty() && !entry.de.is_empty() && entry.text_en.len() > 20 && entry.text_de.len() > 20, "{}", entry.en);
            assert_eq!(HELP.iter().filter(|other| other.en == entry.en).count(), 1, "{} twice", entry.en);
        }
    }

    #[test]
    fn the_german_texts_name_the_tws_with_its_article() {
        for entry in HELP {
            let text = entry.text_de;
            for (at, _) in text.match_indices("TWS") {
                let before = text.get(..at).unwrap_or_default().trim_end();
                let word = before.rsplit([' ', '(']).next().unwrap_or_default();
                let after = text.get(at + 3..).unwrap_or_default();
                let compound = after.starts_with('-') || before.ends_with("IBKR");
                assert!(compound || matches!(word.to_lowercase().as_str(), "die" | "der" | "zur" | "eine"), "\"{word} TWS\" in the help of {}", entry.en);
            }
        }
    }

    #[test]
    fn every_help_text_splits_into_whole_sentences() {
        for entry in HELP {
            for text in [entry.text_en, entry.text_de] {
                let sentences = crate::widgets::sentences(text);
                assert_eq!(sentences.join(" "), text, "nothing is lost: {}", entry.en);
                for sentence in &sentences {
                    assert!(sentence.chars().count() > 12, "\"{sentence}\" of {} is no sentence of its own", entry.en);
                }
            }
        }
    }
}
