import type { LocaleDictionary } from "../types";

const cs: LocaleDictionary = {
  "error.reason.METADATA_GATEWAY_CLOCK_SKEW":
    "Scryer se nemůže připojit k bráně metadat, protože se systémové hodiny liší o více než 5 minut. Synchronizujte systémové hodiny a restartujte Scryer.",
  "httpRelease.refusal.detailsUnavailable":
    "Podrobnosti tohoto vydání už nejsou dostupné. Vyhledejte ho znovu a stáhněte ho.",
  "httpRelease.refusal.live":
    "Toto vydání je živé vysílání, které nelze stáhnout.",
  "httpRelease.refusal.upcoming": "Toto vydání ještě nebylo publikováno.",
  "httpRelease.refusal.removed":
    "Toto vydání bylo odstraněno jeho poskytovatelem.",
  "httpRelease.refusal.unsupportedTransport":
    "Toto vydání není nabízeno ve formě, kterou dokáže HTTP downloader stáhnout.",
  "httpRelease.refusal.separateTracks":
    "Toto vydání je nabízeno pouze jako oddělené zvukové a obrazové stopy, které nelze spojit.",
  "httpRelease.refusal.noMediaResource":
    "Toto vydání neobsahuje žádný mediální soubor ke stažení.",
  "httpRelease.refusal.noSubtitleResource":
    "K tomuto vydání nejsou nabízeny titulky v požadovaném jazyce.",
  "palette.title": "Barevná paleta",
  "palette.description": "Zvolte vzhled pro svůj účet na všech zařízeních.",
  "palette.classic": "Klasický",
  "palette.scryerDefault": "Výchozí Scryer",
  "palette.slate": "Slate",
  "palette.ocean": "Ocean",
  "palette.forest": "Forest",
  "palette.plum": "Plum",
  "palette.ember": "Ember",
  "palette.graphite": "Graphite",
  "palette.papyrus": "Papyrus",
  "palette.classicMode": "Klasický vzhled",
  "palette.choose": "Zvolit barevnou paletu",
  "palette.useAccent": "Použít akcentní barvu palety",
  "palette.saveFailed":
    "Vzhled se nepodařilo uložit. Byl obnoven předchozí výběr.",
  "titleDelete.refusal.selection": "Vyberte tituly bez duplicit.",
  "titleDelete.refusal.busy": "Mazání již probíhá.",
  "titleDelete.refusal.preview": "Obnovte náhled mazání a potvrďte ho.",
  "titleDelete.refusal.confirmation": "Pro potvrzení smazání zadejte DELETE.",
  "titleDelete.refusal.boundary":
    "Složka titulu musí být uvnitř kořenové složky knihovny, ne přímo v ní.",
  "titleDelete.refusal.ownership":
    "Vlastnictví složky se nepodařilo ověřit. Soubory byly zachovány.",
  "titleDelete.refusal.layout":
    "Složku nebo její obsah není bezpečné smazat jako jeden titul.",
  "titleDelete.refusal.unavailable":
    "Kořenová složka knihovny je nedostupná. Obnovte přístup a vytvořte náhled znovu.",
  "titleDelete.refusal.outsideRoots":
    "Cesta k souboru leží mimo všechny nakonfigurované kořenové složky knihoven. Opravte kořenové složky nebo složku titulu a vytvořte náhled znovu.",
  "queue.attention.downloadPasswordRetry":
    "Čeká se na vyřešení opakovaného pokusu s heslem stahování. Pokud klient tuto úlohu už nemá, odeberte toto stahování.",
  "titleDelete.refusal.recycle":
    "Titul byl smazán, ale některé záznamy v koši se nepodařilo vyprázdnit.",
  "titleDelete.refusal.retry":
    "Vyčištění katalogu selhalo. Před dalším pokusem vytvořte nový náhled.",
  "settings.downloadClientHttpDownloadDir": "Složka pro stahování",
  "settings.downloadClientHttpDownloadDirHelp":
    "Kam se zapisují dokončené soubory a jejich rozpracované části, jak je vidí Scryer. Scryer si frontu tohoto downloaderu ukládá ve složce .scryer-http uvnitř této složky.",
  "settings.downloadClientHttpMaxConcurrentDownloads": "Souběžná stahování",
  "settings.downloadClientHttpMaxConcurrentDownloadsHelp":
    "Kolik vydání se stahuje najednou (1 až 8).",
  "settings.downloadClientHttpMaxConnectionsPerHost": "Připojení na hostitele",
  "settings.downloadClientHttpMaxConnectionsPerHostHelp":
    "Kolik stahování smí najednou stahovat ze stejného serveru (1 až 8).",
  "settings.downloadClientHttpBandwidthLimit":
    "Limit přenosové rychlosti (KB/s)",
  "settings.downloadClientHttpBandwidthLimitHelp":
    "Sdílí ho všechna stahování tohoto klienta. Prázdné nebo 0 znamená bez omezení.",
  "settings.publicUrlTitle": "Veřejná URL",
  "settings.publicUrlHelp":
    "Adresa, na které lidé a služby dosáhnou Scryeru, například https://scryer.example.com. Propojování účtů a OAuth používají změnu okamžitě, přístupové klíče (passkey) ji použijí až po restartu Scryeru.",
  "settings.publicUrlLabel": "Veřejná URL",
  "settings.publicUrlEnvironmentLocked":
    "Nastaveno proměnnou SCRYER_PUBLIC_URL. Pro úpravu změňte proměnnou prostředí.",
  "settings.publicUrlInvalid":
    "Nakonfigurovaná veřejná URL je neplatná a nepoužívá se: {{error}}",
  "settings.publicUrlError.invalid_url":
    "Zadejte absolutní URL http nebo https, například https://scryer.example.com.",
  "settings.publicUrlError.wildcard_host":
    "Veřejná URL musí určovat jediného hostitele, ne zástupný vzor.",
  "settings.publicUrlError.credentials":
    "Veřejná URL nesmí obsahovat uživatelské jméno ani heslo.",
  "settings.publicUrlError.query_or_fragment":
    "Veřejná URL nesmí obsahovat parametry dotazu ani fragment.",
  "settings.publicUrlError.path_not_allowed":
    "Scryer je obsluhován z kořene, takže veřejná URL nesmí obsahovat cestu.",
  "settings.publicUrlError.path_mismatch":
    "Cesta ve veřejné URL musí být prázdná nebo odpovídat základní cestě, pod kterou je Scryer obsluhován.",
  "settings.publicUrlError.environment_locked":
    "Veřejná URL je nastavena proměnnou SCRYER_PUBLIC_URL a nelze ji změnit zde.",
  "settings.publicUrlError.save_and_reset":
    "Uložte veřejnou URL, nebo ji vymažte, ne obojí.",
  "settings.publicUrlError.passkey_acknowledgement_required":
    "Tato změna přestane fungovat se zaregistrovanými přístupovými klíči. Pro uložení to potvrďte.",
  "settings.publicUrlPasskeyRestart":
    "Přístupové klíče (passkey) se přizpůsobí změně veřejné URL po restartu Scryeru.",
  "settings.publicUrlClear": "Vymazat veřejnou URL",
  "settings.publicUrlSaved": "Veřejná URL uložena.",
  "settings.publicUrlLoadError": "Veřejnou URL se nepodařilo načíst.",
  "settings.publicUrlSaveError": "Veřejnou URL se nepodařilo uložit.",
  "settings.publicUrlEffectiveValues": "Platné hodnoty",
  "settings.publicUrlColumnSetting": "Nastavení",
  "settings.publicUrlColumnValue": "Hodnota",
  "settings.publicUrlColumnSource": "Zdroj",
  "settings.publicUrlEffective": "Veřejná URL",
  "settings.publicUrlBasePath": "Základní cesta",
  "settings.publicUrlBindAddress": "Vázaná adresa",
  "settings.publicUrlTrustedProxies": "Důvěryhodné proxy servery",
  "settings.publicUrlPasskeyRpId": "ID relying party pro přístupový klíč",
  "settings.publicUrlPasskeyRpOrigin": "Origin přístupového klíče",
  "settings.publicUrlNotSet": "Nenastaveno",
  "settings.publicUrlSourceEnvironment": "Prostředí",
  "settings.publicUrlSourceSettings": "Uložené nastavení",
  "settings.publicUrlSourceDefault": "Výchozí",
  "settings.publicUrlSourcePublicUrl": "Veřejná URL",
  "settings.publicUrlPasskeyWarningTitle":
    "Stávající přístupové klíče přestanou fungovat",
  "settings.publicUrlPasskeyWarningChanged":
    "Přístupové klíče jsou zaregistrovány pro {{rpId}}. Po příštím restartu se přesunou na {{nextRpId}} a každý stávající přístupový klíč přestane fungovat pro přihlášení.",
  "settings.publicUrlPasskeyWarningDisabled":
    "Přístupové klíče jsou zaregistrovány pro {{rpId}}. Tato veřejná URL nemůže nést přístupové klíče (potřebují doménové jméno https nebo localhost), takže po příštím restartu se přístupové klíče vypnou a každý stávající klíč přestane fungovat pro přihlášení.",
  "settings.publicUrlPasskeyWarningCounts":
    "Uživatelé s přístupovými klíči: {{users}}. Bez aplikace autentikátoru jako druhého faktoru: {{passkeyOnly}}. Tito uživatelé budou uzamčeni: nebudou se moci přihlásit, dokud se jejich faktory nenastaví znovu.",
  "settings.publicUrlPasskeyWarningCountsUnknown":
    "Scryeru se nepodařilo spočítat zaregistrované přístupové klíče; počítejte s tím, že se někteří uživatelé uzamknou.",
  "settings.publicUrlPasskeyWarningRecovery":
    "Pro obnovení přístupu může administrátor, který se stále dokáže přihlásit, otevřít {{users}} a pro každého uzamčeného uživatele použít {{reset}}; nebo vrátit veřejnou URL zpět a restartovat Scryer; nebo spustit Scryer s SCRYER_RECOVERY_ADMIN_PASSWORD a přihlásit se jako záchranný administrátor.",
  "settings.publicUrlPasskeyWarningConfirm": "Přesto uložit",
  "settings.pluginSettings": "Nastavení pluginu",
  "settings.pluginSettingsLoadFailed":
    "Nastavení pluginu se nepodařilo načíst.",
  "settings.pluginSettingsLoading": "Načítání nastavení pluginu…",
  "settings.additionalArchivePasswords": "Další hesla",
  "settings.additionalArchivePasswordsHelp":
    "Jedno heslo na řádek. Vyzkouší se, pouze pokud archiv vyžaduje heslo.",
  "settings.pluginSecretSaved":
    "Hodnota je uložena. Zadejte náhradu, nebo ji vymažte.",
  "settings.showEnteredPasswords": "Zobrazit zadaná hesla",
  "settings.hideEnteredPasswords": "Skrýt zadaná hesla",
  "settings.pluginSettingsSaveFailed":
    "Nastavení pluginu se nepodařilo uložit.",
  "settings.regoValidationUnavailable":
    "Validace není dostupná. Zkontrolujte připojení a zkuste Ověřit znovu.",
  "settings.regoValidationFailed": "Ověření pravidla selhalo. Zobrazit chyby.",
  "settings.regoFileAvailability":
    "Dostupné pouze při hodnocení importovaného souboru s mediálními informacemi. Při vyhledávání vydání je input.file null; před čtením polí to ošetřete.",
  "settings.regoExtraAvailability":
    "Atributy poskytnuté indexerem. Klíče se liší podle indexeru a nejsou navrhovány.",
  "settings.regoSnippetLabel": "Úryvek",
  "settings.regoHelper.blockScore":
    "Vrátí standardní blokační penalizaci (-10000). Ostatní položky hodnocení ji mohou stále vyvážit.",
  "settings.regoHelper.sizeGib": "Převede počet bajtů na gibibajty (GiB).",
  "settings.regoHelper.langMatches":
    "Porovná kód jazyka s jiným kódem nebo podporovaným aliasem jazyka.",
  "settings.regoHelper.normalizeSource":
    "Normalizuje popisek zdroje, například webdl na WEB-DL.",
  "settings.regoHelper.normalizeCodec":
    "Normalizuje popisek kodeku, například x265 na H.265.",
  "settings.regoHelper.lower": "Převede řetězec na malá písmena.",
  "settings.regoHelper.upper": "Převede řetězec na velká písmena.",
  "settings.regoHelper.count":
    "Spočítá položky v poli, množině nebo objektu, nebo znaky v řetězci.",
  "settings.regoHelper.objectGet":
    "Přečte klíč objektu a při jeho absenci vrátí výchozí hodnotu. Použijte pojmenovaný vstupní fakt v pravidlech žádostí a údržby.",
  "settings.regoHelper.startsWith": "Vrátí, zda řetězec začíná daným prefixem.",
  "settings.regoSnippet.scoreEntry":
    "Přidá pojmenovanou úpravu skóre, pokud podmínka vydání vyhovuje.",
  "settings.regoSnippet.tagScore":
    "Přidá úpravu skóre pro štítek titulu. Nahraďte zástupné symboly štítku a skóre.",
  "settings.regoSnippet.nullableField":
    "Zkontroluje přítomnost release group, než porovná jeho název v malých písmenech.",
  "settings.regoSnippet.denyRequest":
    "Zamítne žádost, pokud je její fakt o obsahu pro dospělé true.",
  "settings.regoSnippet.manualRequest":
    "Vyžádá ruční kontrolu, jakmile počet nedávných schválení dosáhne prahu.",
  "settings.regoSnippet.matchTag":
    "Shoduje subjekt údržby podle štítku. Tento matcher nedefinuje žádnou akci.",
  "settings.trustedProxiesTitle":
    "Důvěryhodné proxy servery pro omezení četnosti požadavků",
  "settings.trustedProxiesHelp":
    "Při identifikaci klientů pro omezení četnosti požadavků důvěřujte hlavičce X-Forwarded-For pouze z těchto adres proxy. Toto nemění pravidla přihlášení, místní přístup ani výjimky z omezení četnosti. Změny se projeví okamžitě.",
  "settings.trustedProxiesAddresses":
    "Adresy IPv4, IPv6 nebo CIDR (jedna na řádek nebo oddělené čárkou)",
  "settings.trustedProxiesEmptyHelp":
    "Uložení prázdného seznamu znamená, že se nedůvěřuje žádné proxy. Uložené nastavení má přednost před SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS.",
  "settings.trustedProxiesSavedSource": "Zdroj: uložené nastavení",
  "settings.trustedProxiesEnvironmentSource":
    "Zdroj: prostředí (výchozí je bez důvěryhodných proxy)",
  "settings.trustedProxiesReset": "Použít prostředí/výchozí",
  "settings.trustedProxiesSaved": "Politika důvěryhodných proxy byla použita.",
  "settings.trustedProxiesLoadError":
    "Nastavení důvěryhodných proxy se nepodařilo načíst.",
  "settings.trustedProxiesSaveError":
    "Nastavení důvěryhodných proxy se nepodařilo uložit.",
  "mediaInfo.discIntactImport":
    "Obraz disku se importuje jako jeden soubor. Pro epizody namapujte explicitně každý autorský titul.",
  "mediaInfo.discReviewRequired": "Analýza disku vyžaduje kontrolu",
  "mediaInfo.discImportMovie": "Importovat film",
  "mediaInfo.discSkip": "Přeskočit (nepřiřazeno)",
  "mediaInfo.discAutomaticChoice": "Automaticky nejdelší titul",
  "mediaInfo.discNoTitles":
    "Nebyly nalezeny žádné podporované tituly disku. Zdroj zůstane beze změny.",
  "scoring.mandatoryRejection": "Povinný požadavek selhal",
  "scoring.finalScoreRejection": "Konečné skóre zamítnuto",
  "label.ready": "Připraveno",
  "label.moreInfo": "Více informací",
  "label.search": "Hledat",
  "label.autoSearch": "Automatické vyhledávání",
  "label.interactiveSearch": "Interaktivní vyhledávání",
  "label.interactive": "Interaktivní",
  "label.searching": "Vyhledávání",
  "help.autoSearchTooltip":
    "Automaticky vyhledat a zařadit do fronty nejnovější odpovídající vydání této položky.",
  "help.interactiveSearchTooltip":
    "Nyní vyhledat tuto položku a otevřít výběr vydání pro zařazení do fronty.",
  "series.searchSeason": "Vyhledat balíček řady",
  "help.seasonSearchTooltip":
    "Vyhledat na indexerech vydání balíčku pro tuto řadu.",
  "series.interactiveSearchSeason": "Vyhledat řadu",
  "help.seasonInteractiveSearchTooltip":
    "Nyní vyhledat celou tuto řadu a vybrat, které vydání zařadit do fronty.",
  "label.refresh": "Obnovit",
  "label.refreshing": "Obnovování…",
  "label.retry": "Zkusit znovu",
  "label.save": "Uložit",
  "label.saving": "Ukládání…",
  "label.loading": "Načítání…",
  "label.moveUp": "Posunout nahoru",
  "label.moveDown": "Posunout dolů",
  "label.update": "Aktualizovat",
  "label.name": "Název",
  "label.load": "Načíst",
  "label.edit": "Upravit",
  "label.delete": "Smazat",
  "label.account": "Účet",
  "label.actions": "Akce",
  "label.advanced": "Pokročilé",
  "label.custom": "Vlastní",
  "label.create": "Vytvořit",
  "label.cancel": "Zrušit",
  "folderBrowser.selectPath": "Vybrat cestu",
  "folderBrowser.go": "Přejít",
  "folderBrowser.descriptionFileOrFolder":
    "Procházejte cesty na hostiteli Scryeru a vyberte soubor nebo složku.",
  "folderBrowser.descriptionFile":
    "Procházejte cesty na hostiteli Scryeru a vyberte soubor.",
  "folderBrowser.descriptionFolder":
    "Procházejte cesty na hostiteli Scryeru a vyberte složku.",
  "folderBrowser.loading": "Načítání složek",
  "folderBrowser.emptyFilesAndFolders": "Žádné soubory ani podsložky",
  "folderBrowser.emptyFolders": "Žádné podsložky",
  "folderBrowser.error": "Cestu se nepodařilo procházet.",
  "folderBrowser.browseRoot": "Procházet /",
  "folderBrowser.selectFolder": "Vybrat složku",
  "label.discard": "Zahodit změny",
  "label.clear": "Vymazat",
  "label.add": "Přidat",
  "label.remove": "Odebrat",
  "label.yes": "Ano",
  "label.no": "Ne",
  "label.filter": "Filtr",
  "label.filterOptions": "Možnosti filtru",
  "label.noMatchingOptions": "Žádné odpovídající možnosti",
  "label.filters": "Filtry",
  "label.enabled": "Zapnuto",
  "label.disabled": "Vypnuto",
  "unit.seconds": "sekund",
  "label.enable": "Zapnout",
  "label.disable": "Vypnout",
  "label.version": "Verze",
  "label.type": "Typ",
  "label.title": "Název",
  "label.status": "Stav",
  "label.duration": "Doba trvání",
  "label.output": "Výstup",
  "label.testConnection": "Test připojení",
  "external.openOn": "Otevřít na {{site}}",
  "label.unknown": "Neznámé",
  "label.multiEpisodePack": "Balíček více epizod",
  "label.multiEpisodePackHint": "Toto vydání pokrývá víc než hledanou epizodu.",
  "label.unchanged": "Beze změny",
  "label.deleting": "Mazání…",
  "label.default": "výchozí",
  "label.language": "Jazyk",
  "label.movies": "Filmy",
  "label.series": "Seriály",
  "label.anime": "Anime",
  "label.unknownType": "neznámý typ",
  "label.yearUnknown": "rok neznámý",
  "label.noArt": "bez grafiky",
  "label.none": "Žádné",
  "nav.dashboard": "Přehled",
  "nav.movies": "Filmy",
  "nav.series": "Seriály",
  "nav.anime": "Anime",
  "nav.discovery": "Objevování",
  "nav.activity": "Aktivita",
  "nav.calendar": "Kalendář",
  "nav.settings": "Nastavení",
  "nav.library": "Knihovna",
  "nav.catalog": "Katalog",
  "nav.import": "Import",
  "nav.logs": "Protokoly",
  "nav.serviceLogs": "Protokoly služby",
  "nav.auditLogs": "Auditní protokoly",
  "nav.usersAccess": "Uživatelé a přístup",
  "nav.mobileTitle": "Navigace Scryer",
  "nav.mobileDescription": "Hlavní navigace Scryer",
  "nav.mobileTrigger": "Otevřít navigaci",
  "nav.group.overview": "Přehled",
  "nav.group.catalogs": "Katalogy",
  "nav.group.discover": "Objevovat",
  "nav.group.requests": "Žádosti",
  "nav.group.automation": "Automatizace",
  "nav.group.integrations": "Integrace",
  "nav.group.system": "Systém",
  "nav.group.more": "Více",
  "dashboard.commandDescription": "Přehled provozu",
  "dashboard.greetingMorning": "Dobré ráno, {{name}}",
  "dashboard.greetingAfternoon": "Dobré odpoledne, {{name}}",
  "dashboard.greetingEvening": "Dobrý večer, {{name}}",
  "dashboard.attentionCount": "Položek vyžadujících pozornost: {{count}}",
  "dashboard.attentionCountOne": "1 položka vyžaduje pozornost",
  "dashboard.allClear": "Vše běží bez problémů.",
  "dashboard.sourceRequests": "žádosti",
  "dashboard.sourceImports": "importy",
  "dashboard.sourcePlugins": "pluginy",
  "dashboard.sourceIndexers": "indexery",
  "dashboard.pluginUpdateCount": "Aktualizací pluginů: {{count}}",
  "dashboard.pluginUpdateCountOne": "1 aktualizace pluginu",
  "dashboard.pluginBreaking": "Nekompatibilní",
  "dashboard.pluginUpdateAll": "Aktualizovat vše",
  "dashboard.pluginManage": "Spravovat",
  "dashboard.statLibrary": "Knihovna",
  "dashboard.statWindow": "24 h",
  "dashboard.metricMovies": "filmy",
  "dashboard.metricSeries": "seriály",
  "dashboard.metricAnime": "anime",
  "dashboard.metricQueued": "ve frontě",
  "dashboard.metricUpgrades": "upgrady",
  "dashboard.metricImported": "importováno",
  "dashboard.metricFailed": "selhalo",
  "dashboard.deltaHint": "Změna za posledních 24 hodin",
  "dashboard.manualImports": "Ruční importy",
  "dashboard.recentlyImported": "Nedávno importováno",
  "dashboard.storage": "Úložiště",
  "dashboard.activeQueue": "Aktivní fronta",
  "dashboard.viewAll": "Vše",
  "dashboard.approveRequest": "Schválit {{name}}",
  "dashboard.dismissRequest": "Zamítnout {{name}}",
  "dashboard.upgradeBadge": "Upgrade",
  "dashboard.newImportBadge": "Nový import",
  "dashboard.emptyRequests": "Nečekají žádné žádosti.",
  "dashboard.emptyImports": "Žádné importy nevyžadují pozornost.",
  "dashboard.emptyImported": "Zatím nic nebylo importováno.",
  "dashboard.emptyIndexers": "Nejsou nakonfigurovány žádné indexery.",
  "dashboard.emptyClients":
    "Nejsou nakonfigurováni žádní klienti pro stahování.",
  "dashboard.emptyStorage": "Nejsou nakonfigurovány žádné kořenové složky.",
  "dashboard.emptyQueue": "Nic se nestahuje.",
  "dashboard.columnIndexer": "Indexer",
  "dashboard.columnSearch": "Vyhledávání",
  "dashboard.columnGrab": "Stažení",
  "dashboard.columnFail": "Selhání",
  "dashboard.columnQuota": "Kvóta",
  "dashboard.columnStatus": "Stav",
  "dashboard.columnClient": "Klient",
  "dashboard.columnActive": "Aktivní",
  "dashboard.columnQueue": "Fronta",
  "dashboard.healthCount": "{{healthy}}/{{total}} v pořádku",
  "dashboard.erroringCount": "Chybujících: {{count}}",
  "dashboard.erroringCountOne": "1 chybující",
  "dashboard.downCount": "Nedostupných: {{count}}",
  "dashboard.downCountOne": "1 nedostupný",
  "dashboard.unmetered": "Bez měření",
  "dashboard.statusOk": "OK",
  "dashboard.statusError": "Chyba",
  "dashboard.rootCount": "Kořenových složek: {{count}}",
  "dashboard.rootCountOne": "1 kořenová složka",
  "dashboard.storageFree": "volno {{size}}",
  "dashboard.storageUnavailable": "Využití nedostupné",
  "dashboard.storageUsage": "{{used}}/{{total}} TB",
  "dashboard.storageLegend": "OK · 65 %+ · 80 %+ · 90 %+",
  "dashboard.tagLow": "NÍZKÉ",
  "dashboard.tagCrit": "KRIT",
  "dashboard.queueCount": "Aktivních: {{count}}",
  "dashboard.queueCountOne": "1 aktivní",
  "discovery.failedToLoad": "Objevování se nepodařilo načíst",
  "discovery.featured": "Doporučené",
  "discovery.add": "Přidat",
  "discovery.request": "Požádat",
  "discovery.requested": "Požádáno",
  "discovery.openFilters": "Otevřít filtry",
  "discovery.closeFilters": "Zavřít filtry",
  "discovery.filters": "Filtry",
  "discovery.contentType": "Typ obsahu",
  "discovery.type.movies": "Filmy",
  "discovery.type.series": "Seriály",
  "discovery.type.anime": "Anime",
  "discovery.genres": "Žánry",
  "discovery.selectGenres": "Vybrat žánry...",
  "discovery.tags": "Štítky",
  "discovery.selectTags": "Vybrat štítky...",
  "discovery.emptyTitle": "Zatím žádné položky k objevení",
  "discovery.emptyDescription":
    "Kanály objevování se zobrazí, jakmile budou mít data ze synchronizace.",
  "discovery.clearAll": "Vymazat vše",
  "discovery.releaseYear": "Rok vydání",
  "discovery.minimumRating": "Minimální hodnocení",
  "discovery.studio": "Studio",
  "discovery.section.newOnStreaming": "Nově ve streamingu",
  "discovery.section.newOnPhysical": "Nově na Blu-ray",
  "discovery.section.allTimeFavorites": "Oblíbené všech dob",
  "discovery.section.rightNow": "Právě teď",
  "discovery.section.fromYourTaste": "Podle vašeho vkusu",
  "discovery.section.almostComplete": "Téměř dokončené",
  "discovery.section.nextAnimeSeason": "Příští sezóna anime",
  "discovery.section.popularInAnime": "Oblíbené v anime",
  "discovery.section.thisSeasonInAnime": "Tato sezóna v anime",
  "discovery.section.comingSoonMovies": "Již brzy: Filmy",
  "discovery.section.comingSoonSeries": "Již brzy: Seriály",
  "discovery.section.topRatedForYou": "Nejlépe hodnocené pro vás",
  "discovery.section.topRated": "Nejlépe hodnocené",
  "discovery.section.recommended": "Doporučené",
  "discovery.notInterested": "Nemám zájem",
  "discovery.hiddenTitles": "Skryté tituly",
  "discovery.showHidden": "Zobrazit skryté",
  "discovery.updatePending": "Aktualizace čeká",
  "discovery.updatePendingHint":
    "Nové změny v knihovně se zobrazí po příští synchronizaci.",
  "settings.profile": "Profil",
  "settings.general": "Obecné",
  "settings.backups": "Zálohy",
  "settings.security": "Zabezpečení",
  "settings.enableFormLogin": "Zapnout přihlašování",
  "settings.securityLoginSettings": "Nastavení přihlašování",
  "settings.securityEnableFormLoginHelp":
    "Je-li zapnuto, Scryer vyžaduje ověřenou relaci uživatele. Je-li vypnuto, požadavky se řeší jako výchozí administrátorský uživatel.",
  "settings.securityPasswordMinLength": "Minimální délka hesla",
  "settings.securitySessionDuration": "Platnost přihlášení",
  "settings.securitySessionDurationHelp":
    "Platí pro nová přihlášení všech uživatelů na této instanci. Se zapnutou volbou Zůstat přihlášen přežijí relace restart prohlížeče až do vypršení platnosti. Stávající relace si zachovají původní platnost.",
  "settings.securitySessionDurationInvalid":
    "Doba platnosti relace musí být celé číslo od 1 do 365 dnů.",
  "settings.securityPasswordMinLengthHelp":
    "Platí jen pro nová nebo změněná hesla Scryer. Stávající kratší hesla fungují dál. Minimum: {{min}}.",
  "settings.securityPasswordMinLengthInvalid":
    "Minimální délka hesla musí být alespoň {{min}}.",
  "settings.openMediaServers": "Otevřít mediální servery",
  "settings.securitySkipLocalIps": "Přeskočit pro místní IP adresy",
  "settings.securitySkipLocalIpsHelp":
    "Je-li zapnuto, volající z 10.0.0.0/8, 172.16.0.0/12 nebo 192.168.0.0/16 mohou přistupovat ke Scryeru bez přihlášení.",
  "settings.securityMfaConfigStepUp":
    "Vyžadovat vícefaktorové ověření pro změny konfigurace",
  "settings.securityMfaConfigStepUpHelp":
    "Je-li zapnuto, změny systémové konfigurace a konfigurace oblastí vyžadují nedávné ověření vícefaktorovým ověřením.",
  "settings.securityMfaPasswordLogin":
    "Vyžadovat vícefaktorové ověření pro přihlášení heslem",
  "settings.securityMfaPasswordLoginHelp":
    "Je-li zapnuto, přihlášení jménem a heslem Scryer vyžaduje jednorázový kód. Přístupové klíče (passkey) se tím nemění.",
  "settings.securityTotpJellyfinLogin":
    "Vyžadovat vícefaktorové ověření pro přihlášení přes Jellyfin",
  "settings.securityTotpJellyfinLoginHelp":
    "Je-li zapnuto, přihlášení přes přeposlané heslo Jellyfin vyžaduje uživatelův zaregistrovaný přístupový klíč Scryer nebo faktor autentikátoru.",
  "settings.securitySavedPreference": "Uložená volba",
  "settings.securityEffectiveMode": "Aktuálně platný režim",
  "settings.securityOverrideTitle": "Je aktivní přepsání nasazením",
  "settings.securityOverrideDescription":
    "Konfigurace prostředí momentálně přepisuje uloženou volbu. Pro jiný platný režim změňte konfiguraci nasazení a restartujte Scryer.",
  "settings.securityOverrideReason": "Zdroj přepsání: {{override}}",
  "settings.securityModeEnabled": "Přihlašování formulářem zapnuto",
  "settings.securityModeDisabled": "Otevřený administrátorský režim",
  "settings.securityConfirmTitle": "Zapnout přihlašování formulářem?",
  "settings.securityConfirmDescription":
    "Než Scryer začne vyžadovat přihlášení, zadejte své heslo.",
  "settings.securityConfirmPassword": "Heslo",
  "settings.securityConfirmAction": "Zapnout",
  "settings.securitySetAdminPasswordTitle": "Nastavte administrátorské heslo",
  "settings.securitySetAdminPasswordDescription":
    "Váš účet zatím nemá heslo. Zvolte ho nyní a Scryer s ním zapne přihlašování.",
  "settings.securitySetAdminPasswordAction": "Nastavit heslo a zapnout",
  "settings.securitySetAdminPasswordMinLength":
    "Minimální počet znaků: {{count}}.",
  "settings.securityPasswordTooShort":
    "Heslo je příliš krátké. Minimální počet znaků: {{count}}.",
  "settings.securityDisableConfirmTitle": "Vypnout přihlašování formulářem?",
  "settings.securityDisableConfirmDescription":
    "Scryer se po načtení vrátí do otevřeného administrátorského režimu.",
  "settings.securityDisableConfirmAction": "Vypnout",
  "settings.securityCredentialsInvalid": "Neplatné heslo.",
  "settings.securityCredentialsInsufficient":
    "Tento účet nemůže spravovat nastavení.",
  "settings.mfaStepUpTitle": "Ověřte se pro otevření nastavení",
  "settings.securityAccessControl": "Řízení přístupu",
  "profile.apiKeyMfaTitle": "Ověřte se pro vytvoření klíče API",
  "profile.apiKeyMfaDescription":
    "Pro pokračování ve vytváření klíče API zadejte kód z autentikátoru nebo záložní kód.",
  "profile.apiKeyMfaCode": "Kód z autentikátoru nebo záložní kód",
  "settings.mfaStepUpDescription":
    "Před zobrazením nebo změnou nastavení konfigurace zadejte kód z autentikátoru.",
  "settings.mfaStepUpSubmit": "Pokračovat",
  "settings.mfaStepUpVerifying": "Ověřování...",
  "settings.mfaStepUpVerified": "Přístup k nastavení ověřen.",
  "settings.mfaStepUpFailed":
    "Tento kód se nepodařilo ověřit. Zkuste to znovu.",
  "settings.mfaStepUpRequiredAgain":
    "Platnost ověření nastavení vypršela. Pro pokračování zadejte nový kód z autentikátoru.",
  "settings.mfaStepUpPolicyLoadFailed":
    "Stav ověření nastavení se nepodařilo načíst",
  "settings.mfaStepUpPolicyLoadFailedDescription":
    "Před zobrazením nebo změnou nastavení konfigurace to zkuste znovu.",
  "settings.mfaStepUpPolicyRetry": "Zkusit znovu",
  "settings.securityEnabledSuccess": "Přihlašování formulářem zapnuto.",
  "settings.securityDisabledSuccess":
    "Přihlašování formulářem vypnuto. Načítání do otevřeného administrátorského režimu…",
  "settings.securityPreferenceSaved": "Nastavení zabezpečení uloženo.",
  "settings.securityLoadFailed": "Nastavení zabezpečení se nepodařilo načíst.",
  "settings.securitySaveFailed": "Nastavení zabezpečení se nepodařilo uložit.",
  "settings.externalAuthLoginEnabled": "Přihlašování zapnuto",
  "settings.mediaServerLoginRequiresFormLogin":
    "Před zapnutím přihlašování přes mediální server zapněte přihlašování formulářem.",
  "settings.openSecuritySettings": "Otevřít nastavení zabezpečení",
  "settings.externalAuthLinkingEnabled": "Propojování zapnuto",
  "settings.externalAccountInvites": "Pozvánky k externímu účtu",
  "settings.externalAccountInviteRequired":
    "Zvolte uživatele a účet mediálního serveru.",
  "settings.externalAccountInviteCreated":
    "Pozvánka k externímu účtu vytvořena.",
  "settings.externalAccountInviteFailed":
    "Pozvánku k externímu účtu se nepodařilo vytvořit.",
  "settings.mediaServerUser": "Uživatel mediálního serveru",
  "settings.mediaServerUserSearchPlaceholder":
    "Hledat uživatele mediálního serveru",
  "settings.mediaServerUserSearchFailed":
    "Vyhledávání uživatelů mediálního serveru selhalo.",
  "settings.mediaServerUserPickerEmpty":
    "Nebyli nalezeni žádní uživatelé mediálního serveru.",
  "settings.mediaServerUserPickerServerEmpty":
    "Na tomto serveru nebyli nalezeni žádní uživatelé.",
  "settings.previousExternalAccountInvites": "Předchozí pozvánky",
  "settings.noExternalAccountInvites": "Zatím žádné pozvánky k externímu účtu.",
  "settings.externalAccountInviteStatusDisabled": "Vypnuto",
  "settings.externalAccountInviteStatusLoggedIn": "Přihlášen",
  "settings.externalAccountInviteStatusPending": "Čeká se",
  "settings.externalAccountInviteStatusActive": "Aktivní",
  "settings.externalAccountInviteProviderUser": "Uživatel poskytovatele",
  "settings.externalAccountInviteCreatedAt": "Vytvořeno",
  "settings.externalAccountInviteLastLogin": "Poslední přihlášení",
  "settings.provider": "Poskytovatel",
  "settings.createInvite": "Vytvořit pozvánku",
  "settings.users": "Uživatelé",
  "settings.indexers": "Indexery",
  "settings.indexerSearch": "Vyhledávání indexerů",
  "settings.indexerSearchBetaNotice":
    "Vyhledávání indexerů je ve fázi beta. Jeho chování a výsledky se mohou mezi vydáními ještě měnit.",
  "settings.proxies": "Proxy servery",
  "settings.proxyUsername": "Uživatelské jméno",
  "settings.proxyPassword": "Heslo",
  "settings.proxyCredentialsStored": "Přihlašovací údaje nastaveny",
  "settings.proxyCredentialUnchanged": "Beze změny",
  "settings.proxyClearCredentials": "Vymazat uložené přihlašovací údaje",
  "settings.proxyCredentialsHelp":
    "Přihlašovací údaje jsou uloženy zašifrované a už se znovu nezobrazí. Pokud proxy žádné nepotřebuje, nechte obě pole prázdná.",
  "settings.proxyCredentialsStoredHelp":
    "Pro tuto proxy jsou uloženy přihlašovací údaje. Nechte pole prázdná, aby zůstaly zachovány, nebo zadejte nové hodnoty, kterými je nahradíte.",
  "settings.proxyRemoteDns": "Překládat DNS na proxy",
  "settings.proxyRemoteDnsHelp":
    "Překlad DNS na proxy je chování socks5h: proxy vyhledává názvy cílových hostitelů místo tohoto počítače.",
  "settings.proxiesHelp":
    "Proxy servery přenášejí provoz pro indexery a klienty pro stahování, ke kterým je přiřadíte. Řešitel (solver) odpovídá na výzvy prohlížeče, standardní proxy je přechod HTTP nebo SOCKS a tunel vytváří vlastní transport.",
  "settings.proxyFamilySolver": "Řešitel",
  "settings.proxyFamilyStandard": "Standardní",
  "settings.proxyFamilyTunnel": "Tunel",
  "settings.proxyFamilyOther": "Jiné",
  "settings.proxyHealth": "Stav",
  "settings.proxyLastError": "Poslední chyba",
  "settings.proxyHealthHealthy": "V pořádku",
  "settings.proxyHealthUnhealthy": "Nefunkční",
  "settings.proxyEmpty": "Nejsou nastaveny žádné proxy servery.",
  "settings.proxyTest": "Test",
  "settings.proxyTestTitle": "Test {{name}}",
  "settings.proxyTestUrl": "Testovací URL",
  "settings.proxyTestDescription":
    "Načte tuto URL přes uložený proxy server a zobrazí veřejnou IP adresu nahlášenou koncovým bodem.",
  "settings.proxyTestSolverDescription":
    "Požádá solver, aby načetl tuto URL, a zobrazí jeho odchozí IP adresu. Tím se ověří pouze načítání, nikoli řešení výzev ani trasa použitá ostatním provozem Scryeru.",
  "settings.proxyTestRunning": "Testuje se odchozí připojení…",
  "settings.proxyTestExitIp": "Výstupní IP proxy",
  "settings.proxyTestSolverIp": "Výstupní IP solveru",
  "settings.proxyTestNoIp": "Odpověď neobsahovala rozpoznatelnou IP adresu.",
  "settings.proxyTimeout": "Časový limit",
  "settings.proxyCreateNew": "Připojit proxy",
  "settings.proxyCreate": "Vytvořit proxy",
  "settings.proxyUpdate": "Aktualizovat proxy",
  "settings.proxyDeleteConfirmDescription": "Smazat proxy {{name}}?",
  "settings.proxyValidation": "Je vyžadován název a základní URL.",
  "settings.proxyEndpoint": "Koncový bod",
  "settings.proxyEndpointHelp":
    "ssh://host:port. Když port vynecháte, použije se port 22.",
  "settings.proxyAssignment": "Proxy",
  "settings.proxyDirect": "Přímo (bez proxy)",
  "settings.proxyMissing": "Chybějící proxy",
  "settings.proxyMissingHelp": "Přiřazený proxy server nebyl nalezen.",
  "settings.proxyDisabledHelp": "Přiřazený proxy server je vypnutý.",
  "settings.proxyDisabledSuffix": "(vypnuto)",
  "settings.proxyClearPassword": "Vymazat uložené heslo",
  "settings.proxyTunnelAuthHelp":
    "SSH vyžaduje uživatelské jméno a soukromý klíč Ed25519. Ověření heslem není podporováno.",
  "settings.proxyPrivateKey": "Soukromý klíč",
  "settings.proxyPrivateKeyHelp":
    "podporovány jsou pouze soukromé klíče Ed25519; vygenerujte jeden příkazem `ssh-keygen -t ed25519` a vložte soukromý klíč ve formátu OpenSSH",
  "settings.proxyPrivateKeyStored": "Soukromý klíč nastaven",
  "settings.proxyPrivateKeyUnchanged":
    "Soukromý klíč je uložen. Vložte nový, chcete-li jej nahradit.",
  "settings.proxyClearPrivateKey": "Vymazat uložený soukromý klíč",
  "settings.proxyPrivateKeyPassphrase": "Heslová fráze klíče",
  "settings.proxyPrivateKeyPassphraseHelp":
    "Potřeba pouze, pokud je soukromý klíč chráněn heslovou frází.",
  "settings.proxyHostKey": "Klíč hostitele",
  "settings.proxyHostKeyPinnedAt": "Připnuto {{time}}.",
  "settings.proxyHostKeyUnpinned":
    "Zatím nepřipnuto. První úspěšné připojení připne klíč hostitele serveru.",
  "settings.proxyHostKeyReset": "Resetovat klíč hostitele",
  "settings.proxyHostKeyResetDescription":
    "Zapomenout připnutý klíč hostitele. Další připojení důvěřuje a připne cokoli, co server nabídne, takže to udělejte jen tehdy, když víte, že se klíč serveru skutečně změnil.",
  "settings.proxyValidationTunnelUsername":
    "Tunel potřebuje uživatelské jméno.",
  "settings.proxyValidationTunnelAuth":
    "SSH tunel potřebuje soukromý klíč Ed25519.",
  "settings.downloadClientProxyHelp":
    "Lze přiřadit libovolný druh proxy. Solver výzev nemá vliv na nativního klienta, jehož požadavky nevytváří plugin guest.",
  "status.proxyCreated": "Proxy vytvořena.",
  "status.proxyUpdated": "Proxy aktualizována.",
  "status.proxyDeleted": "Proxy {{name}} smazána.",
  "status.proxyTestPassed": "Test proxy úspěšný.",
  "status.proxyTestFailed": "Test proxy se nezdařil.",
  "status.editingProxy": "Úprava proxy {{name}}.",
  "status.proxyHostKeyReset":
    "Klíč hostitele pro {{name}} resetován. Další připojení jej znovu připne.",
  "settings.proxyEndpointHelpHttp3":
    "https://host:port (výchozí 443). Používá HTTP/3 CONNECT přes QUIC s ověřeným TLS. Testujte trasu přiřazením indexeru.",
  "settings.proxyEndpointHelpWireguard":
    "wireguard://host:port. Když port vynecháte, použije se port 51820.",
  "settings.proxyPrivateKeyHelpWireguard":
    "Klíče WireGuard mají 32 bajtů v base64, přesně tak, jak je vypisuje `wg genkey` (44 znaků končících na `=`)",
  "settings.proxyPeerPublicKey": "Veřejný klíč protějšku",
  "settings.proxyPeerPublicKeyHelp":
    "Řádek `PublicKey` ze sekce `[Peer]` serveru. Veřejný klíč je veřejný, proto je zobrazen celý.",
  "settings.proxyPresharedKey": "Předsdílený klíč",
  "settings.proxyPresharedKeyHelp":
    "Volitelné. Řádek `PresharedKey`, pokud jej server používá.",
  "settings.proxyPresharedKeyStored": "Předsdílený klíč nastaven",
  "settings.proxyPresharedKeyUnchanged":
    "Předsdílený klíč je uložen. Vložte nový, chcete-li jej nahradit.",
  "settings.proxyClearPresharedKey": "Vymazat uložený předsdílený klíč",
  "settings.proxyTunnelAddresses": "Adresy",
  "settings.proxyTunnelAddressesHelp":
    "Řádek `Address` ze sekce `[Interface]`, například 10.6.0.2/32. Jedna na řádek, nebo oddělené čárkou.",
  "settings.proxyTunnelDnsServers": "DNS servery",
  "settings.proxyTunnelDnsServersHelp":
    "Volitelné resolvery dostupné přes tunel, jeden na řádek nebo oddělené čárkou. Bez nich musí být cíle adresovány přes IP.",
  "settings.proxyTunnelMtu": "MTU",
  "settings.proxyTunnelMtuHelp":
    "Volitelné, mezi {{min}} a {{max}}. Nechte prázdné pro výchozí hodnotu {{default}}.",
  "settings.proxyTunnelKeepalive": "Keepalive",
  "settings.proxyTunnelKeepaliveHelp":
    "Volitelné, v sekundách. Nechte prázdné pro výchozí hodnotu {{default}}, nebo nastavte 0 pro vypnutí keepalive.",
  "settings.proxyTunnelPublicKey": "Veřejný klíč tohoto tunelu",
  "settings.proxyTunnelPublicKeyHelp":
    "Vložte toto do sekce `[Peer]` serveru jako řádek `PublicKey`.",
  "settings.proxyTunnelPublicKeyPending":
    "Uloží se po uložení soukromého klíče.",
  "settings.proxyTunnelPublicKeyCopy": "Kopírovat",
  "settings.proxyImportConfig": "Importovat konfiguraci WireGuard",
  "settings.proxyImportConfigHelp":
    "Vložte celou konfiguraci od vašeho poskytovatele VPN, nebo vyberte soubor .conf, a pole níže se vyplní z ní. Každé pole také přijme jeden samostatně vložený řádek.",
  "settings.proxyImportConfigApply": "Vyplnit formulář",
  "settings.proxyImportConfigFile": "Vybrat soubor",
  "status.proxyImportConfigFilled": "Formulář vyplněn z konfigurace WireGuard.",
  "status.proxyImportConfigUnreadable":
    "Tohle nevypadá jako konfigurace WireGuard.",
  "status.proxyImportConfigFirstPeer":
    "Počet protějšků: {{count}}; importován byl pouze první.",
  "status.proxyImportConfigIgnored":
    "Ignorováno {{keys}}, které tunelová proxy nepoužívá.",
  "settings.proxyValidationWireguardPrivateKey":
    "WireGuard potřebuje soukromý klíč.",
  "settings.proxyValidationWireguardPeerPublicKey":
    "WireGuard potřebuje veřejný klíč protějšku.",
  "settings.proxyValidationWireguardAddresses":
    "WireGuard potřebuje alespoň jednu adresu rozhraní, například 10.6.0.2/32.",
  "settings.proxyValidationWireguardKeyShape":
    "{{field}} nevypadá jako klíč WireGuard: má 32 bajtů v base64, přesně tak, jak je vypisuje `wg genkey` (44 znaků končících na `=`).",
  "settings.proxyValidationWireguardMtu":
    "MTU tunelu musí být mezi {{min}} a {{max}}.",
  "settings.proxyValidationWireguardKeepalive":
    "Keepalive je celé číslo sekund, hodnota 0 jej vypíná.",
  "status.proxyTunnelPublicKeyCopied": "Veřejný klíč zkopírován.",
  "status.proxyTunnelPublicKeyCopyFailed":
    "Veřejný klíč se nepodařilo zkopírovat.",
  "settings.qualityProfiles": "Profily kvality",
  "settings.downloadClients": "Klienti pro stahování",
  "settings.mediaServers": "Mediální servery",
  "settings.downloadClientRouting": "Směrování klientů pro stahování",
  "settings.moviesSettings": "Nastavení filmů",
  "settings.seriesSettings": "Nastavení seriálů",
  "settings.animeSettings": "Nastavení anime",
  "facetSettings.general": "Obecné",
  "facetSettings.quality": "Kvalita",
  "facetSettings.renaming": "Přejmenování",
  "facetSettings.routing": "Směrování",
  "facetSettings.scoringPersona": "Persona hodnocení",
  "facetSettings.scoringPersonaOverrideLabel": "Přepsání persony",
  "facetSettings.scoringPersonaUseDefault": "Použít výchozí z profilu",
  "facetSettings.scoringBehavior": "Chování hodnocení",
  "facetSettings.scoringBehaviorHint":
    "Jak jsou vydání hodnocena při vyhledávání. Určuje to persona. Jemné ladění proveďte úpravou profilu kvality.",
  "persona.trait.balanced.source":
    "Zdroj: standardní preference (BluRay > WEB-DL > WeRip)",
  "persona.trait.balanced.audio": "Zvuk: vyvážené vážení kodeků",
  "persona.trait.balanced.x265": "x265: penalizováno pod 4K",
  "persona.trait.balanced.size": "Velikost souboru: preferuje větší soubory",
  "persona.trait.balanced.remux": "Remux: standardně nezvýhodněno",
  "persona.trait.balanced.hdr": "HDR: Dolby Vision a HDR10 jsou zvýhodněny",
  "persona.trait.audiophile.source": "Zdroj: silně preferován BluRay",
  "persona.trait.audiophile.audio":
    "Zvuk: silně zvýhodněny bezeztrátové kodeky (TrueHD, DTS-X, FLAC)",
  "persona.trait.audiophile.x265": "x265: silně penalizováno pod 4K",
  "persona.trait.audiophile.size":
    "Velikost souboru: preferovány větší soubory",
  "persona.trait.audiophile.remux": "Remux: velmi vysoký bonus",
  "persona.trait.audiophile.hdr": "HDR: Dolby Vision silně zvýhodněno",
  "persona.trait.efficient.source": "Zdroj: preferováno WEB-DL a WEB-Rip",
  "persona.trait.efficient.audio":
    "Zvuk: ztrátové kodeky akceptovány (preferováno DDP)",
  "persona.trait.efficient.x265": "x265: zvýhodněno při všech rozlišeních",
  "persona.trait.efficient.size":
    "Velikost souboru: preferována kompaktní kódování",
  "persona.trait.efficient.remux": "Remux: nezvýhodněno",
  "persona.trait.efficient.hdr": "HDR: standardní",
  "persona.trait.compatible.source": "Zdroj: preferováno WEB-DL",
  "persona.trait.compatible.audio":
    "Zvuk: DDP a AAC zvýhodněny kvůli široké podpoře zařízení",
  "persona.trait.compatible.x265": "x265: preferováno H.264 před H.265",
  "persona.trait.compatible.size": "Velikost souboru: standardní preference",
  "persona.trait.compatible.remux": "Remux: nezvýhodněno",
  "persona.trait.compatible.hdr":
    "HDR: Dolby Vision penalizováno (riziko kompatibility)",
  "settings.moviesLibrarySettings": "Nastavení mediálních souborů filmů",
  "settings.seriesLibrarySettings": "Nastavení mediálních souborů seriálů",
  "libraryFilter.all": "Všechny knihovny",
  "libraryFilter.selectedCount": "Knihoven: {{count}}",
  "settings.librariesLabel": "Knihovny",
  "settings.libraryNameLabel": "Název knihovny",
  "settings.libraryNamePlaceholder": "Pro děti",
  "settings.libraryNewButton": "Nová knihovna",
  "settings.librarySaveAndScanButton": "Uložit a prohledat",
  "settings.librarySaveOnlyButton": "Jen uložit",
  "settings.libraryDeleteButton": "Smazat knihovnu",
  "settings.libraryDeleteConfirm":
    "Smazat {{name}}? Mediální soubory na disku nebudou odstraněny.",
  "settings.libraryEmpty": "Nejsou dostupné žádné knihovny.",
  "settings.libraryCreated": "Knihovna vytvořena.",
  "settings.librarySaved": "Knihovna uložena.",
  "settings.libraryDeleted": "Knihovna smazána.",
  "settings.librarySaveFailed": "Uložení knihovny se nezdařilo.",
  "settings.libraryDeleteFailed": "Smazání knihovny se nezdařilo.",
  "settings.librarySettingsLoadFailed":
    "Načtení nastavení knihovny se nezdařilo.",
  "settings.libraryRequiredAudioLabel": "Požadovaný zvuk",
  "settings.libraryEffectiveAudio": "Platná hodnota: {{value}}",
  "settings.libraryEffectiveValue": "Platná hodnota: {{value}}",
  "settings.searchLanguages": "Jazyky pro vyhledávání vydání",
  "settings.searchLanguagesHelp":
    "Vyberte jazyky v pořadí vyhledávání. Automatické vyhledávání zkouší tyto názvy, když dřívější dotazy nenajdou přijatelné vydání; interaktivní vyhledávání zkouší všechny vybrané názvy. Jazyky metadat a zvuku se nemění.",
  "settings.searchAliases": "Vlastní názvy vydání",
  "settings.searchAliasesHelp": "Oddělte názvy středníky",
  "settings.libraryMetadataLanguageLabel": "Jazyk metadat",
  "settings.libraryEffectiveMetadataLanguage": "Platná hodnota: {{value}}",
  "settings.librarySeasonFoldersLabel": "Složky řad",
  "settings.libraryEffectiveSeasonFolders": "Platná hodnota: {{value}}",
  "settings.libraryInheritGlobal": "Zdědit globální výchozí hodnotu",
  "settings.useSeasonFoldersLabel": "Používat složky řad",
  "settings.useSeasonFoldersHelp":
    "Umístit soubory epizod do jejich složky řady nebo speciálů.",
  "settings.libraryQualityProfileLabel": "Profil kvality",
  "settings.libraryRequestQualityProfilesLabel": "Profily pro žádosti",
  "settings.libraryRequestQualityProfilesHelp":
    "Nechte prázdné, aby žádosti používaly profil kvality knihovny.",
  "settings.libraryInheritFacet": "Zdědit výchozí hodnotu z katalogu",
  "settings.libraryCustomRouting": "Vlastní směrování knihovny",
  "settings.libraryEffectiveProfile": "Platná hodnota: {{value}}",
  "settings.libraryScoringPersonaLabel": "Persona hodnocení",
  "settings.libraryEffectivePersona": "Platná hodnota: {{value}}",
  "settings.rootFoldersLabel": "Kořenové složky",
  "settings.rootFoldersEmpty":
    "Nejsou nastaveny žádné kořenové složky. Přidejte alespoň jednu cestu.",
  "settings.rootFoldersHelp":
    "Kořenové složky jsou nejvyšší adresáře, kde jsou uložena média. Výchozí složka se používá pro nové tituly.",
  "settings.rootFolderAdd": "Přidat kořenovou složku",
  "settings.rootFolderEdit": "Upravit kořenovou složku",
  "settings.rootFolderConflict":
    "Tuto kořenovou složku už používá {{libraries}}.",
  "settings.rootFolderSetDefault": "Nastavit jako výchozí",
  "settings.rootFolderInvalid": "Neplatná",
  "settings.rootFolderInvalidTooltip":
    "Scryeru se nepodařilo tuto kořenovou složku prohledat z hostitele.",
  "settings.rootFolderValidationUnavailable":
    "Ověření kořenové složky je dočasně nedostupné. Uložené cesty nebyly označeny jako neplatné.",
  "settings.unsavedLibraryChangesTitle": "Neuložené změny knihovny",
  "settings.unsavedLibraryChangesConfirm":
    "Zahodit neuložené změny knihovny a opustit tuto stránku?",
  "settings.libraryScanTitle": "Prohledání knihovny",
  "settings.libraryScanButton": "Prohledat knihovnu",
  "settings.libraryScanRunning": "Prohledává se knihovna…",
  "settings.libraryScanSummary":
    "Importováno: {{imported}}, přeskočeno: {{skipped}}, nepřiřazeno: {{unmatched}}.",
  "settings.libraryScanFailed": "Prohledání knihovny se nezdařilo.",
  "settings.libraryScanCancel": "Zrušit",
  "settings.libraryScanRunInBackground": "Spustit na pozadí",
  "settings.libraryScanCancelFailed":
    "Zrušení prohledání knihovny se nezdařilo.",
  "settings.libraryScanCanceledSummary":
    "Prohledání knihovny zrušeno. Importováno: {{imported}}, přeskočeno: {{skipped}}, nepřiřazeno: {{unmatched}}.",
  "settings.libraryScanToastTitle": "Prohledání knihovny {{facet}}",
  "settings.libraryScanDiscovering": "Zjišťování titulů…",
  "settings.libraryScanProgressPending": "Čeká se na zjištění…",
  "settings.libraryScanProgressCalculatingTotal": "Počítá se celkový počet…",
  "settings.libraryScanProgressScannedCount": "Prohledáno: {{count}}",
  "settings.libraryScanTitleMatch": "Přiřazení titulu",
  "settings.libraryScanFilesScanned": "Analýza médií",
  "settings.libraryScanNoTitleMatchNeeded": "Přiřazení titulu není potřeba",
  "settings.libraryScanNoFilesToScan": "Analýza médií není potřeba",
  "settings.libraryScanAlreadyRunning":
    "Prohledání knihovny {{facet}} už probíhá.",
  "settings.libraryScanProgressCount": "{{current}} / {{total}}",
  "settings.libraryScanScanningSubtitle":
    "Nalezeno titulů: {{count}} · analýza médií…",
  "settings.libraryScanDoneSubtitle":
    "Prohledání dokončeno · titulů: {{count}}",
  "settings.libraryScanIssuesSubtitle":
    "Dokončeno s problémy · titulů: {{count}}",
  "settings.libraryScanFailedSubtitle": "Prohledání se nezdařilo",
  "settings.libraryScanCanceledSubtitle": "Prohledání zrušeno",
  "settings.libraryScanBadgeDone": "Hotovo",
  "settings.libraryScanBadgeReview": "Ke kontrole",
  "settings.libraryScanBadgeFailed": "Selhalo",
  "settings.libraryScanBadgeCanceled": "Zrušeno",
  "settings.libraryScanViewTitles": "Zobrazit titulů: {{count}}",
  "settings.libraryScanReviewUnmatched":
    "Zkontrolovat nepřiřazených: {{count}}",
  "jobs.title": "Úlohy",
  "jobs.schedule": "Plán",
  "jobs.never": "Nikdy",
  "jobs.runSummaryCompleted": "Dokončeno",
  "jobs.runSummaryRunning": "Probíhá",
  "jobs.recentRuns": "Nedávné běhy",
  "jobs.loadingRecentRuns": "Načítání nedávných běhů…",
  "jobs.noRunsYet": "Zatím nebyly zaznamenány žádné běhy.",
  "jobs.healthCheckIssues": "Problémy kontroly stavu",
  "jobs.fullHashBackfillFailures": "Soubory, které se nepodařilo hashovat",
  "jobs.fullHashBackfillFailuresMore": "…a další nezobrazené: {{count}}",
  "jobs.nextRunPrefix": "Příští běh: {{value}}",
  "jobs.startedAt": "Spuštěno {{value}}",
  "jobs.completedAt": "Dokončeno {{value}}",
  "jobs.failedToTrigger": "Spuštění úlohy se nezdařilo",
  "jobs.column.name": "Název",
  "jobs.column.schedule": "Plán",
  "jobs.column.nextRun": "Příští běh",
  "jobs.column.lastRun": "Poslední běh",
  "jobs.column.status": "Stav",
  "jobs.action.run": "Spustit",
  "jobs.action.runNow": "Spustit nyní",
  "jobs.action.runArtworkNow": "Spustit nyní (přepsat plán)",
  "jobs.status.idle": "Nečinná",
  "jobs.status.queued": "Ve frontě",
  "jobs.status.discovering": "Zjišťování",
  "jobs.status.running": "Probíhá",
  "jobs.status.completed": "Dokončeno",
  "jobs.status.warning": "Varování",
  "jobs.status.failed": "Selhalo",
  "jobs.triggerSource.manual": "Ručně",
  "jobs.triggerSource.scheduledStartup": "Naplánováno při spuštění",
  "jobs.triggerSource.scheduledInterval": "Naplánovaný interval",
  "jobs.triggerSource.scheduledDaily": "Naplánováno denně",
  "jobs.triggerSource.systemInternal": "Interní systémová akce",
  "jobs.builtin.title": "Plán úloh",
  "jobs.custom.title": "Vaše úlohy",
  "jobs.custom.description":
    "Skripty, které spouštíte podle plánu nebo na vyžádání.",
  "jobs.custom.add": "Přidat úlohu",
  "jobs.custom.addTitle": "Přidat úlohu",
  "jobs.custom.editTitle": "Upravit úlohu",
  "jobs.custom.editorDescription":
    "Napište skript nebo ukažte na soubor na disku a pak zvolte, kdy se spustí.",
  "jobs.custom.empty":
    "Zatím žádné úlohy. Přidejte skript, aby se spouštěl podle plánu.",
  "jobs.custom.edit": "Upravit úlohu",
  "jobs.custom.enable": "Zapnout úlohu",
  "jobs.custom.disable": "Vypnout úlohu",
  "jobs.custom.disabled": "Vypnuto",
  "jobs.custom.delete": "Smazat úlohu",
  "jobs.custom.capturedOutput": "Zachycený výstup",
  "jobs.custom.noOutput": "Zatím žádné běhy se zachyceným výstupem.",
  "jobs.custom.started": "Spuštěno",
  "jobs.custom.duration": "Doba trvání",
  "jobs.custom.exitCode": "Návratový kód",
  "deletePreview.counting": "Počítání souborů ke smazání...",
  "deletePreview.error": "Náhled mazání se nepodařilo připravit.",
  "deletePreview.folderUnknown":
    "Scryer nepozná, ve které složce jsou soubory tohoto titulu, takže z disku nic nesmaže.",
  "deletePreview.removeFromCatalogOnly": "Odebrat pouze z katalogu",
  "deletePreview.files": "Soubory",
  "deletePreview.media": "Média",
  "deletePreview.subtitles": "Titulky",
  "deletePreview.images": "Obrázky",
  "deletePreview.other": "Ostatní",
  "deletePreview.folders": "Složky",
  "deletePreview.samplePaths": "Ukázkové cesty",
  "deletePreview.confirmPrompt": "Pro potvrzení tohoto mazání napište DELETE.",
  "settings.movieSettingsSaved": "Nastavení filmů uloženo.",
  "settings.seriesSettingsSaved": "Nastavení seriálů uloženo.",
  "settings.qualityProfileSection": "Profil kvality",
  "settings.qualityProfileGlobalLabel": "Výchozí profil kvality (globální)",
  "settings.qualityProfileNameRequired": "Je vyžadován název profilu kvality.",
  "settings.qualityProfileNameDuplicate":
    "Názvy profilů kvality musí být jedinečné.",
  "settings.qualityProfileGlobalHelp":
    "Použije se, když ho kategorie nepřepíše.",
  "settings.qualityProfileOverrideLabel": "Přepsání kvality pro {{category}}",
  "settings.qualityProfileOverridesLabel":
    "Přepsání profilu kvality podle kategorie",
  "settings.qualityProfileOverrideHelp":
    "Zvolte dědit pro návrat ke globálnímu výchozímu nastavení této kategorie obsahu.",
  "settings.qualityProfileInheritLabel": "Zdědit z globálního nastavení",
  "settings.qualityProfileCatalogInvalid":
    "Katalog profilů kvality není platný JSON.",
  "settings.qualityProfileUnknown": "Neznámé ID profilu kvality „{{id}}“.",
  "settings.qualitySettingsSaved": "Nastavení kvality uloženo.",
  "settings.downloadClientRoutingSaved":
    "Směrování klientů pro stahování uloženo.",
  "settings.downloadClientRoutingSavedFor":
    "Směrování klientů pro stahování uloženo pro „{{name}}“.",
  "settings.delayProfiles": "Profily zpoždění",
  "settings.acquisition": "Získávání",
  "settings.acquisitionSaved": "Nastavení získávání uloženo.",
  "settings.acquisitionIntro":
    "RSS je ustálená cesta získávání: každá požadovaná položka je aktivně vyhledána jednou na indexer a pak sledována přes RSS. Aktivní dohledávání je rozvrženo a konečné.",
  "settings.acquisitionEnabled": "Zapnout automatické získávání",
  "settings.acquisitionEnabledHelp":
    "Spouští smyčku získávání na pozadí: konvergenční vyhledávací průchody a párování přes RSS.",
  "settings.acquisitionThresholds": "Prahy pro upgrade",
  "settings.acquisitionSameTierMinDelta":
    "Minimální rozdíl skóre ve stejné úrovni",
  "settings.acquisitionSameTierMinDeltaHelp":
    "Vydání ve stejné úrovni kvality musí překonat skóre aktuálního souboru alespoň o tuto hodnotu.",
  "settings.acquisitionConvergence": "Konvergence a tempo",
  "settings.acquisitionPollIntervalSeconds": "Interval kontroly (sekundy)",
  "settings.acquisitionPollIntervalSecondsHelp":
    "Jak často se u klientů pro stahování kontrolují neúspěšná stahování.",
  "settings.acquisitionWalkIntervalSeconds":
    "Interval vyhledávacího průchodu (sekundy)",
  "settings.acquisitionWalkIntervalSecondsHelp":
    "Jak často se knihovna kontroluje na chybějící a vylepšitelná média a vyhledá se dávka titulů. Přidání titulu, úprava profilu kvality a podobné změny spustí průchod okamžitě. Průchod nikdy neběží častěji než interval kontroly.",
  "settings.acquisitionMaxScopesPerCycle":
    "Max. počet položek vyhodnocených za cyklus",
  "settings.acquisitionMaxScopesPerCycleHelp":
    "Horní hranice vyhodnocení pro jeden cyklus dohledávání. Skutečnou rychlost požadavků určují limity indexeru — ne tento počet.",
  "settings.acquisitionReconvergeDays": "Znovu konvergovat po (dnech)",
  "settings.acquisitionReconvergeDaysHelp":
    "0 to vypíná (doporučeno): konvergované položky jsou sledovány přes RSS. Zapněte pouze jako záložní opakovanou kontrolu, když se ukáže, že je feed neúplný.",
  "settings.movieAvailability": "Dostupnost filmu",
  "settings.libraryMovieAvailabilityDescription":
    "Přepsat nastavení dostupnosti oblasti Filmy pro tuto knihovnu. Každý film si může stále nastavit vlastní minimální dostupnost.",
  "settings.movieAvailabilityFacetDescription":
    "Kdy automatické vyhledávání začne film hledat. Každá knihovna filmů může tato nastavení přepsat a každý film si může nastavit vlastní minimální dostupnost.",
  "settings.movieDefaultAvailability": "Výchozí minimální dostupnost",
  "settings.movieDefaultAvailabilityHelp":
    "Platí pro každý film, který nemá vlastní minimální dostupnost.",
  "settings.movieReleaseMarket": "Trh vydání",
  "settings.movieReleaseMarketHelp":
    "Dvoupísmenný kód země podle ISO-3166 použitý pro výběr kinodistribučního, digitálního a fyzického data.",
  "settings.movieAvailabilityDelay": "Posun dostupnosti (dny)",
  "settings.movieAvailabilityDelayHelp":
    "Záporné hodnoty umožňují dřívější získání; kladné jej zpožďují. Nula začíná v den vydání.",
  "settings.movieAvailabilityStatus.available": "Dostupné",
  "settings.movieAvailabilityStatus.waiting": "Čeká se",
  "settings.movieAvailabilityStatus.unknown": "Neznámé",
  "settings.movieAvailability.known": "Známé datum",
  "settings.movieAvailability.estimated": "Odhadované datum",
  "settings.movieAvailability.reason.available": "Vybraný práh byl splněn.",
  "settings.movieAvailability.reason.release_dates_unknown":
    "Data vydání zatím nejsou známa.",
  "settings.movieAvailability.reason.release_market_refresh_pending":
    "Aktualizují se data pro vybraný trh.",
  "settings.movieAvailability.reason.theatrical_release_date_unknown":
    "Datum kinopremiéry je neznámé.",
  "settings.movieAvailability.reason.home_release_date_unknown":
    "Datum digitálního nebo fyzického vydání je neznámé.",
  "settings.movieAvailability.reason.home_release_date_unusable":
    "Známé datum domácího vydání se nepodařilo přečíst.",
  "settings.movieAvailability.reason.home_release_date_estimated_worldwide":
    "Odhadnuto z nejdřívějšího digitálního nebo fyzického vydání na libovolném trhu.",
  "settings.movieAvailability.reason.home_release_date_estimated_from_worldwide_theatrical":
    "Odhadnuto 90 dní po nejdřívější kinopremiéře na libovolném trhu.",
  "settings.movieAvailability.reason.theatrical_release_date_estimated_worldwide":
    "Odhadnuto z nejdřívější kinopremiéry na libovolném trhu.",
  "settings.movieAvailability.reason.theatrical_release_date_estimated_from_worldwide_home":
    "Odhadnuto z nejdřívějšího domácího vydání na libovolném trhu.",
  "settings.movieAvailability.reason.waiting_for_release_date":
    "Čeká se na datum vydání.",
  "settings.movieAvailability.reason.effective_release_date_out_of_range":
    "Platné datum vydání je mimo rozsah.",
  "settings.delayProfileExisting": "Existující profily zpoždění",
  "settings.delayProfileNone":
    "Nejsou nastaveny žádné profily zpoždění. Vydání budou stahována okamžitě.",
  "settings.delayProfileCreate": "Vytvořit profil",
  "settings.delayProfileCreateNew": "Vytvořit nový profil zpoždění",
  "settings.delayProfileEdit": "Upravit profil",
  "settings.delayProfileConfirmDiscardTitle":
    "Zahodit aktuální úpravy profilu zpoždění?",
  "settings.delayProfileConfirmDiscardDescription":
    "Vaše aktuální úpravy profilu zpoždění budou ztraceny.",
  "settings.delayProfileDeleteConfirm": "Smazat tento profil zpoždění?",
  "settings.delayProfileNameLabel": "Název",
  "settings.delayProfileNamePlaceholder": "např. Počkat na lepší kvalitu",
  "settings.delayProfileNameRequired": "Je vyžadován název profilu zpoždění.",
  "settings.delayProfileUsenetDelay": "Zpoždění Usenetu (minuty)",
  "settings.delayProfileUsenetDelayHelp":
    "Jak dlouho zadržet vydání z Usenetu před stažením. 0 = stáhnout okamžitě.",
  "settings.delayProfileTorrentDelay": "Zpoždění torrentu (minuty)",
  "settings.delayProfileTorrentDelayHelp":
    "Jak dlouho zadržet torrentová vydání před stažením. 0 = stáhnout okamžitě.",
  "settings.delayProfilePreferred": "Preferovaný protokol",
  "settings.delayProfilePreferredHelp":
    "Obejití na základě skóre platí pouze pro vydání na preferovaném protokolu.",
  "settings.delayProfileProtocolModeLabel": "Režim protokolu",
  "settings.delayProfileProtocolModeHelp":
    "Zpoždění se měří od okamžiku, kdy indexer vydání zveřejnil. Preferovaný protokol rozhoduje mezi jinak rovnocennými vydáními a určuje nárok na obejití.",
  "settings.delayProfileProtocolMode.preferUsenet": "Preferovat Usenet",
  "settings.delayProfileProtocolMode.preferTorrent": "Preferovat Torrent",
  "settings.delayProfileProtocolMode.onlyUsenet": "Pouze Usenet",
  "settings.delayProfileProtocolMode.onlyTorrent": "Pouze Torrent",
  "settings.delayProfileMinAge": "Minimální stáří (minuty)",
  "settings.delayProfileMinAgeHelp":
    "Pouze Usenet. Pevná zábrana — vydání mladší než toto jsou zadržena bez ohledu na skóre. Brání stažení neúplných NZB. 0 = vypnuto.",
  "settings.delayProfileBypassLabel": "Práh skóre pro obejití",
  "settings.delayProfileBypassPlaceholder":
    "Nechte prázdné, pokud nikdy neobcházet",
  "settings.delayProfileBypassHelp":
    "Pokud vydání na preferovaném protokolu dosáhne tohoto prahu nebo vyšší hodnoty, stáhne se okamžitě bez ohledu na zpoždění.",
  "settings.delayProfileBypassHighestQualityLabel":
    "Obejít zpoždění pro nejvyšší kvalitu",
  "settings.delayProfileBypassHighestQualityHelp":
    "Stáhne nejkvalitnější vhodné vydání okamžitě, pokud je na preferovaném protokolu.",
  "settings.delayProfileFacetsLabel": "Platí pro",
  "settings.delayProfileFacetsHelp":
    "Pro které typy obsahu tento profil platí. Nechte vše nezaškrtnuté pro všechny typy.",
  "settings.delayProfileLibrariesLabel": "Knihovny",
  "settings.delayProfileLibrariesHelp":
    "Knihovny, pro které tento profil platí. Nechte vybrané všechny, aby platil pro každou knihovnu. Smazáním poslední knihovny profilu se profil vypne.",
  "settings.delayProfileUnknownLibrary": "Neznámá knihovna",
  "settings.delayProfileAllFacets": "Vše",
  "settings.delayProfileTagsLabel": "Štítky",
  "settings.delayProfileTagsPlaceholder":
    "Štítky oddělené čárkou (prázdné = výchozí pro vše ostatní)",
  "settings.delayProfileTagsHelp":
    "Přiřazovat tituly podle štítku. Prázdné znamená, že tento profil je výchozí pro vše ostatní.",
  "settings.delayProfilePriorityLabel": "Priorita",
  "settings.delayProfilePriorityHelp":
    "Nižší číslo = vyšší priorita. Vyhraje první odpovídající profil.",
  "settings.delayProfileEnabledLabel": "Zapnuto",
  "settings.delayProfilesSaved": "Profily zpoždění uloženy.",
  "settings.delayProfileSaveError": "Uložení profilů zpoždění se nezdařilo.",
  "settings.seedingProfiles": "Profily seedování",
  "settings.seedingProfileExisting": "Existující profily seedování",
  "settings.seedingProfileNone":
    "Nejsou nastaveny žádné profily seedování. Torrenty se stahují bez cílů seedování Scryeru a řídí se vlastními limity vašeho klienta pro stahování.",
  "settings.seedingProfileCreate": "Vytvořit profil",
  "settings.seedingProfileCreateNew": "Vytvořit nový profil seedování",
  "settings.seedingProfileEdit": "Upravit profil",
  "settings.seedingProfileConfirmDiscardTitle":
    "Zahodit aktuální úpravy profilu seedování?",
  "settings.seedingProfileConfirmDiscardDescription":
    "Vaše aktuální úpravy profilu seedování budou ztraceny.",
  "settings.seedingProfileDeleteConfirm": "Smazat tento profil seedování?",
  "settings.seedingProfileNameLabel": "Název",
  "settings.seedingProfileNamePlaceholder":
    "např. Soukromý tracker — poměr 1.0",
  "settings.seedingProfileGoalPlaceholder":
    "Nechte prázdné pro použití vlastního limitu klienta",
  "settings.seedingProfileRatioLabel": "Poměr",
  "settings.seedingProfileRatioHelp":
    "Zastavit seedování po dosažení tohoto poměru sdílení. Prázdné ponechává vlastní limit klienta pro stahování.",
  "settings.seedingProfileSeedTimeFormatHint":
    "Doba trvání, například 90m, 36h, 1d 12h nebo 2w. Obyčejné číslo znamená minuty.",
  "settings.seedingProfileSeedTimeLabel": "Čas seedování",
  "settings.seedingProfileMinimumSeedersLabel": "Minimální počet seederů",
  "settings.seedingProfileMinimumSeedersHelp":
    "Vydání z indexeru používajícího tento profil potřebují ke stažení alespoň tento počet seederů. Nechte prázdné pro použití systémové výchozí hodnoty; zadejte 0 pro vypnutí kontroly.",
  "settings.seedingProfileMinimumSeedersUnknownHelp":
    "Indexery, které nehlásí počet seederů, jsou vždy způsobilé.",
  "settings.seedingProfileMinimumSeedersRoutingHelp":
    "Profil přiřazený pouze přes položku směrování klienta pro stahování neuplatňuje své minimum: kontrola proběhne ještě před výběrem klienta, takže se zohlední profil indexeru, pak profil importovaný z Prowlarru a nakonec výchozí profil.",
  "settings.seedingProfileMinimumSeedersInheritHelp":
    "Prázdné pole dědí nastavení Výchozí minimální počet seederů, nikoli hodnotu importovanou z Prowlarru — přiřazením tohoto profilu indexeru spravovanému Prowlarrem se toto importované minimum nahradí.",
  "settings.seedingProfileMinimumSeedersPlaceholder":
    "Nechte prázdné pro použití systémové výchozí hodnoty",
  "settings.seedingProfileMinimumSeedersOffBadge": "Kontrola seederů vypnuta",
  "settings.seedingProfileMinimumSeedersBadge": "Seederů: ≥ {{value}}",
  "settings.seedingMinimumSeedersFloorLabel": "Výchozí minimální počet seederů",
  "settings.seedingMinimumSeedersFloorHelp":
    "Použije se, pokud žádný profil seedování nepokrývá indexer, ze kterého vydání pochází. Zadejte 0 pro povolení vydání bez seederů kdekoli.",
  "settings.seedingMinimumSeedersFloorSaved":
    "Výchozí minimální počet seederů uložen.",
  "settings.seedingProfileSeedTimeHelp":
    "Zastavit seedování po dosažení této doby seedování torrentu. Prázdné ponechává vlastní limit klienta pro stahování.",
  "settings.seedingProfileSeedTimeTransmissionHelp":
    "Transmission nemá celkový limit doby seedování, takže se zde namapuje na jeho limit nečinnosti.",
  "settings.seedingProfileBehaviorColumn": "Chování",
  "settings.seedingProfileSeasonPackSummary": "Balíčky řad: {{value}}",
  "settings.seedingProfileIgnoresMinimums": "Ignoruje minima trackeru",
  "settings.seedingProfileNoneAvailable":
    "Zatím žádné profily — vytvořte jeden v Nastavení → Profily seedování",
  "settings.seedingProfileSeasonPackAdvanced":
    "Přepsání pro balíčky řad (pokročilé)",
  "settings.seedingProfileSeasonPackModeLabel": "Cíle pro balíčky řad",
  "settings.seedingProfileSeasonPackModeHelp":
    "Dědit použije výše uvedené cíle i pro balíčky řad. Přepsat použije místo nich cíle pro balíčky řad.",
  "settings.seedingProfileSeasonPackInherit": "Dědit",
  "settings.seedingProfileSeasonPackOverride": "Přepsat",
  "settings.seedingProfileSeasonPackRatioLabel": "Poměr pro balíček řady",
  "settings.seedingProfileSeasonPackSeedTimeLabel":
    "Čas seedování pro balíček řady",
  "settings.seedingProfileSeasonPackGoalsHelp":
    "Použito pouze v režimu přepsání. Prázdné pole ponechává tento cíl na klientovi pro stahování.",
  "settings.seedingProfileHonorTrackerMinimumsLabel":
    "Respektovat minima trackeru",
  "settings.seedingProfileHonorTrackerMinimumsHelp":
    "Zvýší výsledné cíle na minimální poměr a čas seedování, které vydání deklaruje, aby soukromé trackery nezaznamenaly hit and run.",
  "settings.seedingProfilePostImportTrackingLabel": "Po importu",
  "settings.seedingProfilePostImportTrackingHelp":
    "Zda Scryer po naimportování souborů dál spravuje torrent.",
  "settings.seedingProfilePostImportTrackingPark":
    "Pokračovat ve správě, dokud nejsou splněny cíle",
  "settings.seedingProfilePostImportTrackingHandOff":
    "Předat klientovi pro stahování",
  "settings.seedingProfilePostImportTrackingHandOffBadge": "Předáno",
  "settings.seedingProfilePostImportTrackingHandOffHelp":
    "Po importu Scryer přestane tento torrent spravovat: položka zůstává v klientovi, nikdy se automaticky neodebere a opustí frontu. Cíle seedování ani akce po splnění cíle se už neuplatní — převezmou to vlastní limity vašeho klienta. Soubory jsou stále importovány přes hardlink nebo kopii, zatímco torrent seeduje.",
  "settings.seedingProfileGoalMetActionLabel": "Po splnění cíle",
  "settings.seedingProfileGoalMetActionHelp":
    "Co se stane s torrentem po dosažení jeho cíle seedování.",
  "settings.seedingProfileGoalMetRemoveEntry": "Odebrat položku",
  "settings.seedingProfileGoalMetStopSeeding": "Zastavit seedování",
  "settings.seedingProfileGoalMetKeep": "Pokračovat v seedování",
  "settings.seedingProfileNeverRemoveLabel": "Nikdy neodebírat",
  "settings.seedingProfileNeverRemoveHelp":
    "Blokuje veškeré automatické odebírání torrentů stažených pod tímto profilem, i po splnění cíle. Musíte je odebrat sami.",
  "settings.seedingProfileNeverRemoveBadge": "Nikdy neodebráno",
  "settings.seedingProfileDefaultTitle": "Výchozí profil seedování",
  "settings.seedingProfileDefaultLabel":
    "Platí, když nic konkrétnějšího neodpovídá",
  "settings.seedingProfileDefaultHelp":
    "Použije se pro stažení torrentů, když ani indexer, ani položka směrování klienta pro stahování profil neuvádí.",
  "settings.seedingProfileDefaultNone":
    "Žádný výchozí — použít vlastní limity klienta",
  "settings.seedingProfileDefaultBadge": "Výchozí",
  "settings.seedingProfileMissing": "Chybějící profil „{{id}}“",
  "settings.seedingProfileSaved": "Profil seedování uložen.",
  "settings.seedingProfileSaveError": "Uložení profilu seedování se nezdařilo.",
  "settings.seedingProfileDeleted": "Profil seedování smazán.",
  "settings.seedingProfileDeleteError":
    "Smazání profilu seedování se nezdařilo.",
  "settings.seedingProfileDefaultSaved": "Výchozí profil seedování uložen.",
  "settings.seedingProfileProwlarrManaged": "Spravováno Prowlarrem",
  "settings.seedingProfileProwlarrManagedMinimum":
    "Spravováno Prowlarrem · min. seederů: {{count}}",
  "settings.seedingProfileProwlarrManagedMinimumOne":
    "Spravováno Prowlarrem · minimálně 1 seeder",
  "settings.seedingProfileProwlarrManagedMinimumOff":
    "Spravováno Prowlarrem · kontrola seederů vypnuta",
  "settings.seedingProfileInherit": "Zdědit výchozí",
  "settings.seedingProfileInheritProwlarrMinimum":
    "Zdědit výchozí · Prowlarr, min. seederů: {{count}}",
  "settings.seedingProfileInheritProwlarrMinimumOne":
    "Zdědit výchozí · Prowlarr minimálně 1 seeder",
  "settings.seedingProfileInheritProwlarrMinimumOff":
    "Zdědit výchozí · kontrola seederů Prowlarru vypnuta",
  "settings.seedingProfileRoutingInherit": "Zdědit globální výchozí",
  "settings.seedingProfileColumn": "Profil seedování",
  "settings.seedingProfileIndexerLabel": "Profil seedování pro {{name}}",
  "settings.seedingProfileRoutingLabel": "Profil seedování pro {{name}}",
  "settings.seedingProfileNotApplicable": "Pouze torrentové indexery",
  "status.indexerSeedingProfileSaving": "Ukládání profilu seedování…",
  "status.indexerSeedingProfileSaved":
    "Profil seedování uložen pro „{{name}}“.",
  "settings.mediaServersSection": "Připojení mediálních serverů",
  "settings.mediaServerCreate": "Uložit mediální server",
  "settings.mediaServerCreateNew": "Připojit nový mediální server",
  "settings.mediaServerUpdate": "Aktualizovat mediální server",
  "settings.mediaServerConfirmDiscardTitle":
    "Zahodit aktuální úpravy mediálního serveru?",
  "settings.mediaServerConfirmDiscardDescription":
    "Aktuální úpravy mediálního serveru budou ztraceny.",
  "settings.mediaServerEnabled": "Zapnout připojení mediálního serveru",
  "settings.mediaServerAuthCapabilities": "Oprávnění",
  "settings.mediaServerAutoAddEnabled":
    "Automaticky přidávat nové ověřené uživatele",
  "settings.mediaServerPathMappings": "Mapování cest",
  "settings.mediaServerPathMappingsHelp":
    "Každé pravidlo se ukládá jako cesta na serveru => cesta ve Scryeru.",
  "settings.mediaServerValidation": "Název a základní URL adresa jsou povinné.",
  "settings.noMediaServersFound":
    "Nebyla nalezena žádná připojení mediálních serverů.",
  "settings.capabilities": "Možnosti",
  "settings.credentials": "Přihlašovací údaje",
  "settings.apiKeyConfigured": "Klíč API uložen",
  "settings.apiKeyMissing": "Chybí klíč API",
  "settings.plexServerSelected": "Server Plex vybrán",
  "settings.plexServerMissing": "Server Plex není vybrán",
  "settings.discoverPlexServers": "Přihlaste se k Plexu a vyhledejte servery",
  "settings.plexServer": "Server Plex",
  "settings.plexServerDiscoveryEmpty": "Plex nevrátil žádné dostupné servery.",
  "settings.plexServerDiscoveryRequired":
    "Před zapnutím přihlašování přes Plex, propojení nebo automatického přidávání se přihlaste k Plexu a vyberte server.",
  "settings.setupViaApiKey": "Nastavit pomocí klíče API",
  "settings.loginAsAdmin": "Přihlásit se jako správce",
  "settings.clearSavedApiKey": "Vymazat uložený klíč API",
  "settings.adminUsername": "Uživatelské jméno správce",
  "settings.adminPassword": "Heslo správce",
  "settings.downloadClientCreate": "Připojit klienta pro stahování",
  "settings.downloadClientCreateNew": "Připojit klienta pro stahování",
  "settings.downloadClientUpdate": "Aktualizovat klienta pro stahování",
  "settings.downloadClientConfirmDiscardTitle":
    "Zahodit aktuální úpravy klienta pro stahování?",
  "settings.downloadClientConfirmDiscardDescription":
    "Aktuální úpravy klienta pro stahování budou ztraceny.",
  "settings.downloadClientUseSsl": "Použít SSL",
  "settings.downloadClientUrlBase": "Základní cesta URL",
  "settings.downloadClientUrlPreview": "Vypočtená základní URL adresa",
  "settings.downloadClientCategory": "Kategorie",
  "settings.downloadClientRecentPriority": "Priorita nedávné fronty",
  "settings.downloadClientOlderPriority": "Priorita starší fronty",
  "settings.downloadClientPriorityForce": "Vynutit",
  "settings.downloadClientPriorityVeryHigh": "Velmi vysoká",
  "settings.downloadClientPriorityHigh": "Vysoká",
  "settings.downloadClientPriorityNormal": "Normální",
  "settings.downloadClientPriorityLow": "Nízká",
  "settings.downloadClientPriorityVeryLow": "Velmi nízká",
  "settings.downloadClientPriority": "Priorita",
  "settings.downloadClientRemoveCompleted": "Odebrat dokončené",
  "settings.downloadClientRemoveFailed": "Odebrat neúspěšné",
  "settings.downloadClientUseSslInfo":
    "Při připojování k tomuto klientovi pro stahování použít SSL/TLS.",
  "settings.downloadClientEnabledInfo":
    "Když je vypnuto, tento klient se při směrování stahování přeskočí.",
  "settings.downloadClientEnabledLabel": "Zapnout klienta pro stahování",
  "settings.downloadClientFilesystemPathMapping":
    "Mapování cesty v souborovém systému (pokročilé)",
  "settings.downloadClientFilesystemPathMappingHelp":
    "Mapování vzdálených cest je potřeba jen zřídka. Použijte je pouze pokud Scryer a tento klient pro stahování vidí stejnou složku stahování na různých cestách v souborovém systému.",
  "settings.downloadClientRemotePathMappings": "Mapování vzdálených cest",
  "settings.downloadClientRemotePathMappingsHelp":
    "Každé pravidlo se ukládá jako VZDÁLENÁ => LOKÁLNÍ.",
  "settings.downloadClientRemotePathMappingsRemoteLabel": "Vzdálená cesta",
  "settings.downloadClientRemotePathMappingsRemoteHelp":
    "Cesta hlášená klientem pro stahování.",
  "settings.downloadClientRemotePathMappingsLocalLabel": "Lokální cesta",
  "settings.downloadClientRemotePathMappingsLocalHelp":
    "Cesta, kterou by měl Scryer na tomto počítači používat.",
  "settings.downloadClientRemotePathMappingsAdd": "Přidat mapování",
  "settings.downloadClientRemotePathMappingsEmptyState":
    "Zatím nejsou nastavena žádná mapování cest. Přidejte jedno pouze pokud Scryer nedokáže přečíst cestu hlášenou klientem pro stahování.",
  "settings.downloadClientRemotePathMappingsExampleLabel": "Příklad",
  "settings.downloadClientRemotePathMappingsRemoteRequired":
    "Pokud je řádek mapování přítomen, je vzdálená cesta povinná.",
  "settings.downloadClientRemotePathMappingsLocalRequired":
    "Pokud je řádek mapování přítomen, je lokální cesta povinná.",
  "settings.downloadClientRemotePathMappingsLocalAbsolute":
    "Lokální cesta musí být absolutní.",
  "settings.downloadClientRemotePathMappingsRemoteDuplicate":
    "Vzdálená cesta duplikuje jiné mapování u tohoto klienta.",
  "settings.downloadClientSabnzbdAuthHelp":
    "SABnzbd podporuje buď klíč API, nebo uživatelské jméno a heslo. U Decypharru to obvykle je host :8282, základní URL /sabnzbd, uživatelské jméno = vaše URL Arr, heslo = váš token Arr.",
  "settings.downloadClientSabnzbdNzbdavHelp":
    "U NzbDav nasměrujte dokončená stahování kompatibilní se SAB na připojenou cestu completed-symlinks, ujistěte se, že Scryer dokáže toto připojení číst lokálně, a pokud NzbDav hlásí jinou viditelnou cestu, přidejte mapování vzdálených cest.",
  "settings.downloadClientQbittorrentDecypharrHelp":
    "U qBittorrentu s Decypharrem použijte jako uživatelské jméno svou URL Arr a jako heslo svůj token Arr. Ve Směrování klientů pro stahování pro danou oblast nastavte odpovídající název Arr, například radarr nebo sonarr.",
  "settings.downloadClientDecypharrFilesystemHelp":
    "Import dokončených stahování funguje jen z cest, které dokáže Scryer číst lokálně. U Decypharru a NzbDav Scryer importuje ze složky dokončených symlinků hlášené klientem, takže tuto cestu připojte do Scryeru a pokud se hlášené cesty liší, přidejte mapování vzdálených cest.",
  "settings.downloadClientNamePlaceholder": "Můj klient pro stahování",
  "settings.downloadClientHostPlaceholder": "download.example.com",
  "settings.downloadClientUrlBasePlaceholder": "např. api",
  "settings.downloadClientPortPlaceholder": "8080",
  "settings.downloadClientCategoryPlaceholder": "series",
  "settings.downloadClientStatus": "Stav",
  "settings.downloadClientLastError": "Chyba",
  "settings.downloadClientLastSeen": "Naposledy viděn {{time}}",
  "settings.downloadClientNoActivity": "Žádná aktivita",
  "settings.downloadClientRoutingScope":
    "Metadata klienta pro stahování pro {{scope}}",
  "settings.downloadClientRoutingGloballyEnabled": "Globálně zapnuto",
  "settings.downloadClientRoutingEnabled": "Zapnuto",
  "settings.noDownloadClientsFound":
    "Nebyli nalezeni žádní klienti pro stahování.",
  "settings.indexerRoutingScope": "Metadata indexeru pro {{scope}}",
  "settings.indexerRoutingPriority": "Priorita",
  "settings.indexerRoutingCategories": "Kategorie",
  "settings.indexerRoutingGloballyEnabled": "Globálně zapnuto",
  "settings.indexerRoutingEnabled": "Zapnuto",
  "settings.indexerRoutingCategoriesPlaceholder": "Vyberte kategorie",
  "settings.indexerRoutingNoIndexers": "Nebyly nalezeny žádné indexery.",
  "settings.indexerRoutingCustomCategories": "Vlastní kódy",
  "settings.indexerRoutingCustomCategoryPlaceholder":
    "Číselné ID z možností indexeru",
  "settings.indexerRoutingCustomCategoryAdd": "Přidat",
  "settings.indexerRoutingIndexerCategories": "Také na tomto indexeru",
  "settings.indexerCategorySeries": "Seriály",
  "settings.indexerCategoryMovies": "Filmy",
  "settings.indexerCategoryOther": "Ostatní",
  "settings.indexerCategoryForeign": "Zahraniční",
  "settings.indexerCategorySd": "SD",
  "settings.indexerCategoryHd": "HD",
  "settings.indexerCategoryUhd": "UHD",
  "settings.indexerCategorySport": "Sport",
  "settings.indexerCategoryAnime": "Anime",
  "settings.indexerCategoryDocumentary": "Dokument",
  "settings.indexerCategoryBluRay": "Blu-ray",
  "settings.indexerCategoryThreeD": "3D",
  "settings.indexerCategoryMisc": "Různé",
  "indexerCategory.labelCategory": "Kategorie",
  "settings.downloadClientValidation": "Název a hostitel jsou povinné.",
  "settings.downloadClientBaseUrlRequired":
    "Jsou vyžadovány platný hostitel a port.",
  "settings.folderTemplateLabel": "Šablona složky",
  "settings.folderTemplatePlaceholder": "Zástupný symbol",
  "settings.folderTemplateRequired": "Šablona složky je povinná.",
  "settings.seasonFolderTemplateLabel": "Šablona složky řady",
  "settings.seasonFolderTemplatePlaceholder": "Řada {season}",
  "settings.seasonFolderTemplateRequired": "Šablona složky řady je povinná.",
  "settings.specialsFolderTemplateLabel": "Šablona složky speciálů",
  "settings.specialsFolderTemplatePlaceholder": "Speciály",
  "settings.specialsFolderTemplateRequired":
    "Šablona složky speciálů je povinná.",
  "settings.seasonFolderTemplateMustContainSeason":
    "Šablona složky řady musí obsahovat {season}.",
  "settings.seasonFolderValidationUnknownToken":
    "Šablony složek řad podporují {season}, title, year a tokeny externích ID. Neplatný token: {{token}}",
  "settings.folderRenameSectionTitle": "Přejmenování složek",
  "settings.folderValidationUnknownToken":
    "Šablony složek podporují {title}, {year} a tokeny externích ID. Neplatný token: {{token}}",
  "settings.folderValidationEmpty": "Šablona složky nesmí být prázdná",
  "settings.folderValidationLeafTitleToken":
    "Poslední složka v šabloně musí obsahovat token titulu nebo externího ID",
  "settings.folderValidationIllegalCharacter":
    "Šablona složky obsahuje nepovolený znak souborového systému: {{character}}",
  "settings.renameTemplateLabel": "Šablona přejmenování",
  "settings.renameTemplatePlaceholder": "Zástupný symbol",
  "settings.renameTemplateRequired": "Šablona přejmenování je povinná.",
  "settings.renameCollisionPolicyLabel": "Zásada kolizí",
  "settings.renameCollisionPolicySkip": "Přeskočit",
  "settings.renameCollisionPolicyError": "Chyba",
  "settings.renameCollisionPolicyReplaceIfBetter": "Nahradit, pokud je lepší",
  "settings.renameMissingMetadataPolicyLabel": "Zásada chybějících metadat",
  "settings.renameMissingMetadataPolicyFallbackTitle":
    "Použít jako náhradu název",
  "settings.renameMissingMetadataPolicySkip": "Přeskočit",
  "settings.renameSectionTitle": "Přejmenování souborů",
  "settings.renameReferenceTitle": "Reference",
  "settings.renameReferenceTokens": "Tokeny",
  "settings.renameReferenceFunctions": "Funkce",
  "settings.renameReferenceFiles": "Soubory",
  "settings.renameReferenceFolders": "Složky",
  "settings.renameLiteralBracesLabel": "Doslovné složené závorky",
  "settings.renameFunctionTruncateTitle": "Délka",
  "settings.renameFunctionOptionalGroupTitle": "Volitelná skupina",
  "settings.renameOptionalGroupDescription":
    "Skupinu vykreslit pouze pokud má její řídicí token hodnotu. Volitelné skupiny nelze vnořovat.",
  "settings.renameFunctionWhitespaceTitle": "Mezery",
  "settings.renameFunctionWhitespaceDescription":
    "K výběru oddělovače pro text tokenu použijte jeden filtr mezer.",
  "settings.renameFunctionPaddingTitle": "Doplnění čísel",
  "settings.renameFunctionChainTitle": "Řetězení",
  "settings.renameFunctionBracesTitle": "Složené závorky",
  "settings.renameTruncateFilterLabel": "Omezit token na prvních N znaků.",
  "settings.renameNumberPaddingLabel": "Doplnit číselné tokeny na N číslic.",
  "settings.renameSpaceUnderscoreFilterLabel": "Nahradit mezery podtržítky.",
  "settings.renameSpaceDotFilterLabel": "Nahradit mezery tečkami.",
  "settings.renameSpaceDashFilterLabel": "Nahradit mezery pomlčkami.",
  "settings.renameSpaceRemoveFilterLabel": "Odebrat mezery z hodnoty tokenu.",
  "settings.renameFilterChainLabel": "Spustit filtry zleva doprava.",
  "settings.renameTokenTitle": "Název titulu",
  "settings.renameTokenYear": "Rok vydání",
  "settings.renameTokenTitleWithYear":
    "Název titulu následovaný rokem, přidaným pouze pokud jím název ještě nekončí",
  "settings.renameTokenTitleWithoutYear":
    "Název titulu s odstraněným rokem na konci",
  "settings.renameTokenTitleWithArticle":
    "Titul s úvodním členem přesunutým na konec podle jazyka jeho metadat",
  "settings.renameTokenTitleFirstCharacter":
    "První písmeno nebo číslice po jazykově citlivém řazení členů; zachovává nelatinské znaky a jinak použije _",
  "rename.namingLanguage": "Jazyk pojmenování: {{language}}",
  "rename.organizingFolderChanges":
    "Titulů měnících organizační složku: {{count}}",
  "settings.renameTokenQuality": "Kvalita (např. 1080p)",
  "settings.renameTokenEdition": "Edice (např. Director's Cut)",
  "settings.renameTokenImdbId": "IMDb ID",
  "settings.renameTokenTmdbId": "TMDb ID",
  "settings.renameTokenTvdbId": "TVDb ID",
  "settings.renameTokenAnidbId": "AniDB ID",
  "settings.renameTokenMalId": "MAL ID",
  "settings.renameTokenAnilistId": "AniList ID",
  "settings.renameTokenSource": "Zdroj (např. BluRay, WEB-DL)",
  "settings.renameTokenVideoCodec": "Video kodek (např. x265)",
  "settings.renameTokenAudioCodec": "Zvukový kodek (např. DTS)",
  "settings.renameTokenAudioChannels": "Zvukové kanály (např. 5.1)",
  "settings.renameTokenSeason": "Číslo řady",
  "settings.renameTokenSeasonOrder": "Pořadí řady (narativní pozice)",
  "settings.renameTokenEpisode": "Číslo epizody",
  "settings.renameTokenAbsoluteEpisode": "Absolutní číslo epizody",
  "settings.renameTokenEpisodeTitle": "Název epizody",
  "settings.renameTokenGroup": "Vydávající skupina",
  "settings.renameTokenExt": "Přípona souboru",
  "settings.renameValidationUnmatchedOpen": "Nepárová otevírací závorka '{'",
  "settings.renameValidationUnmatchedClose": "Nepárová zavírací závorka '}'",
  "settings.renameValidationUnknownToken": "Neznámý token: {{token}}",
  "settings.renameValidationInvalidPadding":
    "Doplnění čísel musí být celé číslo ASCII od 0 do 240: {{padding}}",
  "settings.renameValidationInvalidFilter": "Neplatný filtr tokenu: {{filter}}",
  "settings.renameValidationEmpty": "Šablona přejmenování nesmí být prázdná",
  "settings.renameValidationInvalidOptionalGroup": "Neplatná volitelná skupina",
  "settings.renameValidationNestedOptionalGroup":
    "Volitelné skupiny nelze vnořovat",
  "settings.renameValidationUnsupportedOptionalFallback":
    "Volitelné skupiny nepodporují náhradní větve",
  "settings.mediaSettingsSaved": "Nastavení médií uloženo.",
  "rename.previewButton": "Náhled přejmenování",
  "rename.previewing": "Vytváření náhledu…",
  "rename.applyButton": "Použít přejmenování",
  "rename.applying": "Používání…",
  "rename.currentPath": "Aktuální",
  "rename.proposedPath": "Navrhované",
  "rename.planSummary":
    "Položek: {{total}}, k přejmenování: {{renamable}}, beze změny: {{noop}}, konfliktů: {{conflicts}}, chyb: {{errors}}",
  "rename.noRenamableFiles": "Žádné soubory nevyžadují přejmenování.",
  "rename.blockedFiles":
    "Souborů, které nelze přejmenovat: {{count}}. Kvůli nim nelze přejmenovat tento titul.",
  "rename.sampleNote":
    "Zobrazeno prvních {{shown}} z {{renamable}} souborů k přejmenování.",
  "settings.sectionTitle": "Nastavení • {{section}}",
  "settings.generalText": "Obecná nastavení aplikace.",
  "settings.generalPlaceholder": "Nastavte jazyk a další globální předvolby.",
  "settings.dateTimeFormatLabel": "Formát data a času",
  "settings.dateTimeFormatLocale": "Výchozí dle oblasti",
  "settings.dateTimeFormatIso24h": "RRRR-MM-DD, 24hodinový čas",
  "settings.dateTimeFormatHelp":
    "Určuje, jak jsou v rozhraní zobrazována data a časy.",
  "settings.historyRetentionTitle": "Uchovávání historie",
  "settings.historyRetentionHelp":
    "Toto zachovává uživatelsky viditelnou historii a auditní stopu Scryeru. Změny se projeví při příštím běhu údržby.",
  "settings.historyRetentionExternalHelp":
    "Provozní úklid dočasných dat získávání stále probíhá a toto nijak neovlivňuje historii uchovávanou SABnzbdem, NZBGetem nebo Weaverem.",
  "settings.featuresHeader": "Funkce",
  "settings.apiExplorerLabel": "Zapnout API Explorer",
  "settings.apiExplorerHelp":
    "Přidá GraphiQL do navigační položky API uživatelům s oprávněním Správa systémových nastavení. Požadavky se spouští s oprávněními klíče API nebo knihovny OAuth.",
  "apiExplorer.accessMode": "Přístup jako",
  "apiExplorer.renameTab": "Přejmenovat kartu",
  "apiExplorer.tabName": "Název karty",
  "apiExplorer.apiKey": "Klíč API",
  "apiExplorer.oauth": "Aplikace OAuth",
  "apiExplorer.apiKeyHelp":
    "Vaše oprávnění aplikace a knihovny bez interaktivního přístupu k účtu. Předplatné není dostupné.",
  "apiExplorer.oauthHelp":
    "Pouze vaše oprávnění knihovny. Nastavení aplikace a operace s účtem nejsou dostupné.",
  "apiExplorer.sessionHelp":
    "Použije vaši aktuální relaci. Není potřeba žádné nastavení přihlašovacích údajů ani klienta.",
  "settings.experimentalFeaturesLabel": "Zapnout experimentální funkce",
  "settings.experimentalFeaturesHelp":
    "Experimentální funkce se mohou kdykoliv změnit bez upozornění. Snažíme se zachovat kompatibilitu s předchozími změnami, ale nemůžeme ji zaručit.",
  "settings.personalizedDiscoveryLabel": "Personalizované objevování",
  "settings.personalizedDiscoveryHelp":
    "Odesílá přehled vaší knihovny do brány metadat, aby mohlo Objevování doporučovat tituly podle toho, co vlastníte. Vypněte, aby Objevování zůstalo jen u veřejných zdrojů.",
  "settings.srrdbFilenameRecoveryLabel":
    "Obnovit zamaskované názvy souborů ze srrdb.com",
  "settings.srrdbFilenameRecoveryHelp":
    "Když automatický import ze SABnzbd nebo NZBGet obsahuje video soubory se zamaskovanými názvy, Scryer odešle kontrolní součet CRC-32 a velikost každého souboru na srrdb.com, službu třetí strany, aby vyhledala původní název souboru. Nic jiného o vaší knihovně ani o souboru se neodesílá. Soubory na disku se nikdy nepřejmenovávají.",
  "discovery.personalizedDisabled":
    "Personalizované objevování je pro tuto instanci vypnuto.",
  "settings.keepHistoryForever": "Uchovávat historii navždy",
  "settings.historyRetentionDaysHeader": "Uchovávat historii po dobu",
  "settings.historyRetentionDaysSuffix": "dní",
  "settings.securitySessionDurationSuffix": "dní",
  "settings.generalSaved": "Obecná nastavení uložena.",
  "settings.verificationDepthTitle": "Ověřování kopie při importu",
  "settings.verificationDepthHelp":
    "Jak důkladně Scryer ověří zkopírovaný soubor, než se dotkne zdroje.",
  "settings.verificationDepthScope":
    "Toto platí pouze pro kopie při importu z klienta pro stahování. Přesuny knihovny a kořenové složky se vždy ověřují plně, bez ohledu na tuto volbu.",
  "settings.verificationDepthLabel": "Hloubka ověření",
  "settings.verificationDepthFull": "Plné zpětné čtení",
  "settings.verificationDepthFullHelp":
    "Kopii celou zpětně přečte a porovná ji s kontrolním součtem streamovaným během kopírování.",
  "settings.verificationDepthQuick": "Rychlá kontrola (výchozí)",
  "settings.verificationDepthQuickHelp":
    "Porovná velikost souboru a vzorek obsahu ze začátku a konce. Rychlejší, ale slabší záruka.",
  "settings.verificationDepthSaved": "Hloubka ověření uložena.",
  "settings.uiSaved": "Nastavení zobrazení uložena.",
  "settings.historyRetentionValidation":
    "Počet dní uchovávání historie musí být alespoň 1, pokud historie není uchovávána navždy.",
  "settings.imageCacheTitle": "Mezipaměť obrázků",
  "settings.imageCacheMaxSizeLabel": "Velikost mezipaměti obrázků",
  "settings.imageCacheMbSuffix": "MiB",
  "settings.imageCacheEnvOverride":
    "SCRYER_IMAGE_CACHE_MAX_BYTES je aktivní. Platí jeho hodnota; uložený limit se použije po odstranění přepsání z prostředí.",
  "settings.imageCacheMaxSizeValidation":
    "Maximální velikost mezipaměti obrázků musí být alespoň 1 MiB.",
  "settings.clearImageCache": "Vymazat mezipaměť obrázků",
  "settings.clearingImageCache": "Mazání mezipaměti obrázků…",
  "settings.imageCacheClearQueued":
    "Reset mezipaměti obrázků zařazen do fronty.",
  "settings.pluginHttpTrustTitle": "Důvěryhodné certifikáty HTTPS pro pluginy",
  "settings.pluginHttpTrustAdvancedLabel": "Pokročilá funkce",
  "settings.pluginHttpTrustStoredCount": "Uloženo: {{count}}",
  "settings.pluginHttpTrustDescription":
    "Rozšiřte odchozí důvěru HTTPS pro pluginy WASM o další soukromé nebo vlastní certifikační autority.",
  "settings.pluginHttpTrustWarningTitle":
    "Použijte pouze pokud důvěřujete certifikační autoritě",
  "settings.pluginHttpTrustWarningBody":
    "Tyto certifikáty rozšiřují důvěru HTTPS pro odchozí požadavky pluginů. Nahrávejte pouze soubory certifikačních autorit, které kontrolujete nebo jim výslovně důvěřujete.",
  "settings.pluginHttpTrustUploadButton": "Nahrát důvěryhodné certifikáty",
  "settings.pluginHttpTrustClearButton": "Vymazat vše",
  "settings.pluginHttpTrustEmpty":
    "Nebyly nahrány žádné další důvěryhodné certifikáty.",
  "settings.pluginHttpTrustEntryLabel": "Důvěryhodný certifikát",
  "settings.pluginHttpTrustUploadMissingCertificate":
    "Nahraný soubor neobsahoval blok certifikátu PEM ani certifikát DER.",
  "settings.pluginHttpTrustUploadTrailingText":
    "Nahraný soubor PEM obsahoval neočekávaný text mimo certifikát.",
  "settings.pluginHttpTrustUploadInvalidCertificate":
    "Nahraný certifikát se nepodařilo zpracovat.",
  "settings.pluginHttpTrustUploadReadError":
    "Nahraný soubor certifikátu se nepodařilo zpracovat.",
  "settings.languageChangeTitle": "Změnit jazyk",
  "settings.languageChangeWarning":
    "Změna na {{language}} znovu stáhne metadata pro všechny tituly ve vaší knihovně (názvy, popisy, řadicí názvy) v novém jazyce. Rozhraní se změní okamžitě, ale změna veškerého obsahu v daném jazyce může trvat až 24 hodin.",
  "settings.languageChangeConfirm": "Změnit jazyk",
  "settings.metadataRehydrationStarted":
    "Obnovení metadat zahájeno. Tituly se budou aktualizovat na pozadí.",
  "theme.label": "Motiv",
  "theme.light": "Světlý",
  "theme.dark": "Tmavý",
  "theme.system": "Systémový",
  "theme.switchLabel": "Přepnout motiv (aktuální: {{theme}})",
  "search.globalPlaceholder": "Hledat filmy, seriály, anime, akce a nastavení…",
  "search.globalPlaceholderNoLibrary":
    "Hledat filmy, seriály, anime, akce a nastavení…",
  "search.overlayPlaceholder":
    "Hledat ve vašich knihovnách, akcích, nastavení a všude jinde…",
  "search.overlayPlaceholderNoLibrary":
    "Hledat filmy, seriály, anime, akce a nastavení…",
  "search.shortcutHint": "⌘K",
  "search.shortcutHintControl": "Ctrl+K",
  "search.title": "Globální vyhledávání",
  "search.subtitle":
    "Hledat filmy, seriály, anime, vaši knihovnu, akce a nastavení.",
  "search.subtitleNoLibrary": "Hledat filmy, seriály, anime, akce a nastavení.",
  "search.subtitleWithQuery": "Výsledky pro „{{query}}“",
  "search.statusLoading": "Vyhledávání „{{query}}“…",
  "search.statusResultOne": "1 výsledek pro „{{query}}“",
  "search.statusResultOther": "Výsledků pro „{{query}}“: {{count}}",
  "search.statusNoResults": "Žádné shody pro „{{query}}“",
  "search.tabAll": "Vše",
  "search.tabLibrary": "V knihovně",
  "search.inLibrary": "V knihovně",
  "search.actionsAndSettings": "Akce a nastavení",
  "search.alreadyInCollection": "již ve vaší sbírce",
  "search.resultCountOne": "1 výsledek",
  "search.resultCountOther": "Výsledků: {{count}}",
  "search.monitored": "Sledováno",
  "search.unmonitored": "Nesledováno",
  "search.view": "Zobrazit",
  "search.viewAll": "Zobrazit vše",
  "search.add": "Přidat",
  "search.addToAnotherLibrary": "Přidat do jiné knihovny",
  "search.requestForAnotherLibrary": "Požádat pro jinou knihovnu",
  "search.chooseLibraryToView":
    "Vyberte knihovnu, ve které chcete zobrazit {{name}}",
  "search.footerTip":
    "Nemůžete to najít? Zkuste jiný výraz nebo zkontrolujte své indexery.",
  "search.searchTips": "Tipy pro vyhledávání",
  "search.tipTitles":
    "Hledejte podle názvu filmu, seriálu nebo anime, názvu katalogu, akce nebo nastavení.",
  "search.tipTitlesNoLibrary":
    "Hledejte podle názvu filmu, seriálu nebo anime, akce nebo nastavení.",
  "search.tipTabs":
    "Pomocí karet zúžíte výsledky na vaši knihovnu nebo jeden typ katalogu.",
  "search.tipTabsNoLibrary":
    "Pomocí karet zúžíte výsledky na jeden typ katalogu.",
  "search.tipIndexers":
    "Před kontrolou indexerů na tituly, které nejsou ve výsledcích metadat, použijte širší výraz.",
  "search.noMatchesFor": "Žádné shody pro „{{query}}“",
  "search.emptyHint":
    "Zkuste jiný film, seriál, název anime, katalogu, akci nebo nastavení.",
  "search.emptyHintNoLibrary":
    "Zkuste jiný film, seriál, anime, akci nebo nastavení.",
  "search.minimumQueryTitle": "Pokračujte v psaní",
  "search.minimumQueryHint":
    "Pro vyhledávání ve vaší knihovně a metadatech zadejte alespoň 2 znaky.",
  "search.minimumQueryHintNoLibrary":
    "Pro vyhledávání v metadatech zadejte alespoň 2 znaky.",
  "search.noCatalogMatches": "Zatím žádné shody ve spravovaném katalogu.",
  "search.noMetadataMatches": "V této sekci zatím žádné shody v metadatech.",
  "search.alreadyCataloged": "Již v katalogu",
  "search.cataloged": "V katalogu",
  "search.unavailable": "Nedostupné",
  "search.configureAdd": "Nastavit možnosti přidání",
  "search.addConfigLibrary": "Knihovna",
  "search.addConfigQualityProfile": "Profil kvality",
  "search.addConfigInheritLibrary": "Zdědit od knihovny",
  "search.addConfigRootFolder": "Kořenová složka",
  "search.addConfigSeasonFolder": "Složka řady",
  "search.addConfigMinAvailabilityNoReleaseDate":
    "Datum vydání zatím není známo. Automatické vyhledávání počká, dokud nebude známo.",
  "search.addConfigMonitorType": "Typ sledování",
  "search.addConfigNoQualityProfiles":
    "Nejsou nastaveny žádné profily kvality.",
  "search.seasonFolder.enabled": "Zapnuto",
  "search.seasonFolder.disabled": "Vypnuto",
  "search.monitorType.monitored": "Sledováno",
  "search.monitorType.unmonitored": "Nesledováno",
  "search.monitorType.futureEpisodes": "Budoucí epizody",
  "search.monitorType.missingAndFutureEpisodes": "Chybějící a budoucí epizody",
  "search.monitorType.allEpisodes": "Všechny epizody",
  "search.monitorType.none": "Žádné",
  "search.monitorType.advanced": "Pokročilé",
  "monitorSelection.seasons": "Řady",
  "monitorSelection.seriesMovies": "Filmy ze seriálu",
  "monitorSelection.specials": "Speciály",
  "monitorSelection.season": "Řada {{number}}",
  "monitorSelection.seasonNamed": "Řada {{number}}: {{name}}",
  "monitorSelection.selectAll": "Vybrat vše",
  "monitorSelection.clear": "Vymazat",
  "monitorSelection.retry": "Zkusit znovu",
  "monitorSelection.loading": "Načítání řad…",
  "monitorSelection.loadFailed": "Řady pro tento titul se nepodařilo načíst.",
  "monitorSelection.noSeasons": "Pro tento titul nebyly vráceny žádné řady.",
  "monitorSelection.unavailable":
    "Pokročilé sledování vyžaduje shodu v TVDB, kterou tento výsledek nemá.",
  "monitorSelection.emptyHint":
    "Vyberte alespoň jednu řadu nebo film ze seriálu.",
  "monitorSelection.summaryPrefix": "Požadováno: {{selection}}",
  "monitorSelection.summaryMovies": "Filmy: {{movies}}",
  "search.adding": "Přidávání…",
  "search.request": "Požádat",
  "search.requesting": "Odesílání žádosti…",
  "search.facetMovie": "Film",
  "search.facetSeries": "Seriál",
  "search.facetAnime": "Anime",
  "search.metadataSearch": "Vyhledávání metadat",
  "toast.catalogAdded": "Přidáno do katalogu",
  "toast.catalogAddedAutoSearch": "Automatické vyhledávání zařazeno do fronty.",
  "toast.releaseGrabbed": "Vydání zařazeno ke stažení",
  "toast.titleImported": "Titul importován",
  "toast.viewInCatalog": "Zobrazit v katalogu",
  "label.back": "Zpět",
  "label.close": "Zavřít",
  "label.dismiss": "Zavřít",
  "pwa.offline": "Jste offline. Některé funkce nemusí být dostupné.",
  "pwa.installApp": "Nainstalovat Scryer",
  "pwa.iosInstallHint":
    "Klepněte na tlačítko sdílení a vyberte „Přidat na plochu“ pro instalaci.",
  "smgUpgrade.deprecatedTitle": "Doporučujeme aktualizovat Scryer",
  "smgUpgrade.deprecatedBody":
    "Podpora této verze Scryeru se blíží konci. Pro další používání Scryeru brzy aktualizujte.",
  "smgUpgrade.blockedTitle": "Je vyžadována aktualizace Scryeru",
  "smgUpgrade.blockedBody":
    "Tato verze Scryeru již není podporována. Pro další používání Scryeru aktualizujte nyní.",
  "smgUpgrade.minimumVersion": "Minimální podporovaná verze: {{version}}",
  "smgUpgrade.deadline": "Konec podpory: {{date}}",
  "smgUpdate.title": "Je k dispozici aktualizace Scryeru",
  "smgUpdate.body": "Používáte {{current}}. Nejnovější verze je {{latest}}.",
  "smgUpdate.releaseNotes": "Poznámky k vydání",
  "title.addAndQueue": "Přidat titul a zařadit do fronty",
  "title.addToCatalog": "Přidat do katalogu",
  "title.name": "Název titulu",
  "title.namePlaceholder": "Hledat v TVDB podle názvu",
  "title.facet": "Oblast",
  "title.manageMovies": "Spravovat filmy",
  "title.manageSeries": "Spravovat seriály",
  "title.manageAnime": "Spravovat anime",
  "title.monitored": "Sledováno",
  "title.unmonitored": "Nesledováno",
  "title.ended": "Ukončeno",
  "title.continuing": "Pokračuje",
  "title.upcoming": "Připravuje se",
  "title.backToFacet": "Zpět na {{facet}}",
  "title.notFound": "Titul nenalezen.",
  "title.noPoster": "Žádný plakát",
  "title.addedAt": "Přidáno {{date}}",
  "title.monitorAction": "Sledovat",
  "title.unmonitorAction": "Přestat sledovat",
  "title.renameAction": "Přejmenovat",
  "title.filterPlaceholder": "Filtrovat spravované tituly",
  "title.loading": "Žádná data nenačtena",
  "title.noManaged": "Pro tuto sekci nebyly nalezeny žádné tituly.",
  "title.statusTemplate": "Načteno titulů: {{count}}.",
  "title.contextPanelTitle": "Podrobnosti titulu",
  "title.contextTitleList": "Seznam titulů",
  "title.contextListDisclosure": "Seznam",
  "title.showTitleList": "Zobrazit seznam",
  "title.hideTitleList": "Skrýt seznam",
  "title.contextForYouTitle": "Objevovat",
  "title.contextForYouTop": "Nejlepší",
  "title.contextForYouTopMoviesThisWeek": "Nejlepší filmy tohoto týdne",
  "title.contextForYouTopSeriesThisWeek": "Nejlepší seriály tohoto týdne",
  "title.contextForYouTopAnimeThisWeek": "Nejlepší anime tohoto týdne",
  "title.contextForYouGenre": "Protože máte rádi {{genre}}",
  "title.contextForYouTag": "Protože máte rádi {{tag}}",
  "title.contextForYouAcclaimed": "Oceňováno - není ve vaší knihovně",
  "title.contextForYouCompleteCollection": "Dokončit sbírku",
  "title.contextForYouReasonTop": "Silná shoda s knihovnou",
  "title.contextForYouReasonWeekly": "Týdenní signál z knihovny",
  "title.contextForYouReasonGenre": "Odpovídá vaší oblibě žánru {{genre}}",
  "title.contextForYouReasonTag": "Odpovídá vaší oblibě štítku {{tag}}",
  "title.contextForYouReasonAcclaimed":
    "Vysoce hodnoceno a není ve vaší knihovně",
  "title.contextForYouReasonCollection": "Součást sbírky, kterou vlastníte",
  "title.contextForYouEmptyTitle": "Zatím žádná doporučení",
  "title.contextForYouEmptyBody":
    "Přidejte nebo prohledejte tituly, aby se toto zobrazení knihovny naplnilo.",
  "title.topBilledCast": "Hlavní obsazení",
  "title.dubCast": "Obsazení dabingu",
  "title.dubCastLanguage": "Jazyk",
  "title.contextMoreLikeThis": "Další podobné",
  "title.contextBlockedReleases": "Blokovaná vydání",
  "title.clearBlockedRelease": "Odebrat {{releaseName}} z blokovacího seznamu",
  "title.clearBlockedReleaseHint":
    "Odebrat toto vydání z blokovacího seznamu, aby mohlo být znovu staženo",
  "title.contextAdded": "Přidáno",
  "title.columns": "Sloupce",
  "title.toggleColumns": "Přepnout sloupce",
  "title.table.groupCore": "Základní",
  "title.table.groupRatings": "Hodnocení",
  "title.table.groupMedia": "Média",
  "title.table.groupOperational": "Provozní",
  "title.table.facet": "Oblast",
  "title.table.library": "Knihovna",
  "title.table.monitored": "Sledováno",
  "title.table.qualityTier": "Kvalita",
  "title.table.profile": "Profil",
  "title.table.episodes": "Epizody",
  "title.table.year": "Rok",
  "title.table.runtime": "Délka",
  "title.table.avgRuntime": "Prům. délka",
  "title.table.episodeProgressTooltip": "Epizody: {{owned}} / {{total}}",
  "title.table.episodeProgressTooltipWithTotal":
    "Epizody: {{owned}} / {{total}} (sledováno: {{monitored}})",
  "title.table.status": "Stav",
  "title.table.root": "Kořen",
  "title.table.popularity": "Popularita",
  "title.table.resolution": "Rozlišení",
  "title.table.hdr": "HDR",
  "title.table.audioCodec": "Audio kodek",
  "title.table.size": "Velikost",
  "title.viewModeToggle": "Režim zobrazení",
  "title.viewModeCompact": "Kompaktní",
  "title.viewModePosterTable": "Seznam",
  "title.viewModePoster": "Zobrazení plakátů",
  "title.bulkSelectionCount": "Vybráno: {{count}}",
  "title.bulkSelectionPanelTitle": "Hromadný výběr",
  "title.bulkSelectionPanelHint":
    "Zvolte akci pro vybrané tituly, nebo výběr zrušte.",
  "title.bulkEditTitle": "Upravit vybrané tituly",
  "title.bulkEditDescription": "Použít změny na vybrané tituly ({{count}}).",
  "title.editOptionsTitle": "Upravit možnosti titulu",
  "title.editOptionsDescription": "Změnit možnosti pro {{name}}.",
  "title.bulkDeleteTitle": "Smazat vybrané tituly",
  "title.bulkDeleteDescription":
    "Odebrat vybrané tituly ({{count}}) z katalogu.",
  "title.bulkRenameTitle": "Přejmenovat vybrané tituly",
  "title.bulkRenameDescription":
    "Zobrazit náhled a použít přejmenování pro vybrané tituly ({{count}}).",
  "title.selectTitle": "Vybrat {{name}}",
  "title.selectAllTitles": "Vybrat všechny zobrazené tituly",
  "title.seasonMonitored": "Řada sledována",
  "title.episodeMonitored": "Epizoda sledována",
  "title.seriesMovieMonitored": "Film v seriálu sledován",
  "title.deleteFilesOnDisk": "Trvale smazat soubory z disku",
  "title.seasonsAndEpisodes": "Řady a epizody",
  "title.noTrackedSeasons":
    "Pro tento seriál zatím nejsou sledovány žádné řady.",
  "title.fetchingData": "Načítání dat",
  "title.filesOnDisk": "Soubory na disku",
  "title.files": "Soubory",
  "title.noFilesTracked": "Nejsou sledovány žádné soubory.",
  "title.noFilesTrackedHint":
    "Spusťte kontrolu knihovny, abyste zjistili soubory na disku.",
  "title.configureRootFoldersHint":
    "Před kontrolou této knihovny nastavte kořenové datové složky v nastavení oblasti.",
  "title.invalidRootFoldersTitle":
    "Jedna nebo více kořenových složek je neplatná.",
  "title.invalidRootFoldersHint":
    "Před spuštěním kontroly knihovny opravte nakonfigurované kořenové datové složky v nastavení oblasti.",
  "title.configureRootFoldersButton": "Nastavit kořenové datové složky",
  "title.descriptionUnavailable": "Popis není k dispozici.",
  "title.canon": "Kánon",
  "title.mixed": "Smíšené",
  "title.specials": "Speciály",
  "title.seasonNumber": "Řada {{number}}",
  "title.episodeRange": "Epizody {{start}} – {{end}}",
  "title.episodeCountOne": "{{count}} epizoda",
  "title.episodeCountOther": "Epizod: {{count}}",
  "title.searchingReleases": "Hledání vydání na indexerech…",
  "title.contextReleaseSearchSummary":
    "{{releaseCount}} vydání · prohledáno indexerů: {{indexerCount}}",
  "title.contextReleaseSearchProgress":
    "{{releaseCount}} vydání · indexery: {{done}}/{{total}}",
  "title.contextReleaseSearchSummaryDetailed":
    "{{releaseCount}} vydání · prohledáno indexerů: {{searched}}/{{total}}",
  "title.contextReleaseSearchIndexerFailures":
    "Selhalo indexerů: {{count}} ({{names}})",
  "title.contextReleaseSearchIndexerSkipped":
    "Vynecháno indexerů: {{count}} ({{names}})",
  "title.noReleasesFound": "Pro {{name}} nebyla nalezena žádná vydání.",
  "title.searchNeedsDownloadClientTitle":
    "Vyhledávání vyžaduje klienta pro stahování",
  "title.searchNeedsDownloadClientDescription":
    "Před použitím vyhledávání nebo interaktivního vyhledávání na této stránce titulu nastavte alespoň jednoho klienta pro stahování.",
  "title.searchNeedsDownloadClientAction": "Otevřít klienty pro stahování",
  "title.blockedReleases": "Blokovaná vydání",
  "title.noBlockedReleases":
    "Pro tento film nejsou zaznamenána žádná blokovaná vydání.",
  "title.qualityProfile": "Profil kvality",
  "title.rootFolder": "Kořenová složka",
  "title.catalogFilters.allRootFolders": "Všechny kořenové složky",
  "title.catalogFilters.selectedCount": "Vybráno: {{count}}",
  "title.catalogFilters.minimumYear": "Minimální rok vydání",
  "title.catalogFilters.maximumYear": "Maximální rok vydání",
  "title.catalogFilters.loadError": "Možnosti filtru se nepodařilo obnovit.",
  "title.catalogFilters.retry": "Zkusit znovu načíst možnosti filtru",
  "title.defaultRootFolder": "Výchozí ({{path}})",
  "title.inheritDefault": "Zdědit výchozí",
  "title.monitoredSection": "Sledované: {{facet}}",
  "title.searchReleasesAction": "Hledat vydání",
  "title.queueLatest": "Zařadit nejnovější do fronty",
  "title.requiredAudioLanguages": "Povinné jazyky zvuku",
  "title.originalAudioLanguagePerTitle": "Původní jazyk (podle titulu)",
  "title.requiredAudioLanguagesFacetInfo":
    "Tyto jazyky se vynucují při výběru vydání a při ověřování importu pro tuto oblast.",
  "title.requiredAudioResetInherit": "Obnovit na dědění",
  "title.fixMatchHeading": "Opravit shodu metadat",
  "title.fixMatchAction": "Opravit shodu",
  "title.fixMatchDescriptionMovie":
    "Opravit shodu s TVDB pro tento film. Místní název a pojmenování složky zůstanou stejné.",
  "title.fixMatchDescriptionSeries":
    "Opravit shodu s TVDB pro tento titul. Místní název a pojmenování složky zůstanou stejné.",
  "title.fixMatchDialogTitle": "Opravit shodu",
  "title.fixMatchDialogDescription":
    "Opravit shodu s TVDB pro {{name}}. Místní název a název složky zůstanou stejné. U seriálů a anime se interně znovu sestaví řady a epizody.",
  "title.fixMatchUnnamed": "tento titul",
  "title.fixMatchSearchPlaceholder": "Hledat tituly na TVDB",
  "title.fixMatchCurrentTvdbId": "Aktuální ID TVDB",
  "title.fixMatchCurrentTvdbNone": "žádné",
  "title.fixMatchSearching": "Vyhledávání metadat…",
  "title.fixMatchNoResults": "Nebyly nalezeny žádné shody metadat.",
  "title.fixMatchSelected": "Vybráno",
  "title.fixMatchChoose": "Vybrat",
  "title.fixMatchApplying": "Opravování shody…",
  "title.fixMatchApply": "Použít shodu",
  "title.fixMatchSearchFailed": "Vyhledání metadat se nezdařilo.",
  "title.fixMatchApplyFailed": "Opravení shody titulu se nezdařilo.",
  "title.changeFolderHeading": "Změnit složku",
  "title.changeFolderDescription":
    "Opravte, kterou existující složku tento titul vlastní. Žádné soubory se nepřesouvají.",
  "title.changeFolderAction": "Změnit složku",
  "title.changeFolderDialogTitle": "Změnit složku",
  "title.changeFolderDialogDescription":
    "Vyberte složku, která patří k {{name}}. Scryer přiřadí vlastnictví a znovu zkontroluje složku; žádné soubory se nepřesouvají.",
  "title.changeFolderLibrary": "Knihovna",
  "title.changeFolderCurrentFolder": "Aktuální složka",
  "title.changeFolderCurrentFolderUnknown": "Zobrazí se po výběru složky",
  "title.changeFolderCurrentRoot": "Aktuální kořen",
  "title.changeFolderCurrentRootUnknown": "Neznámý",
  "title.changeFolderRootLabel": "Kořen",
  "title.changeFolderRootScopeHelp":
    "Vybrat lze pouze složky uvnitř kořenových složek knihovny tohoto titulu.",
  "title.changeFolderNoRoots":
    "Knihovna tohoto titulu nemá žádné kořeny k procházení.",
  "title.changeFolderBrowseLoading": "Načítání složek…",
  "title.changeFolderBrowseEmpty": "Žádné podsložky zde nejsou.",
  "title.changeFolderBrowseFailed": "Výpis složek se nezdařil.",
  "title.changeFolderUp": "O úroveň výš",
  "title.changeFolderOpen": "Otevřít",
  "title.changeFolderSelectPrompt": "Vyberte složku pro náhled změny.",
  "title.changeFolderPreviewLoading": "Kontrola vybrané složky…",
  "title.changeFolderPreviewFailed": "Náhled změny složky se nezdařil.",
  "title.changeFolderSelectedFolder": "Vybraná složka",
  "title.changeFolderOwnershipUnowned": "Tuto složku nevlastní žádný titul.",
  "title.changeFolderOwnershipThisTitle":
    "Tento titul již tuto složku vlastní.",
  "title.changeFolderOwnershipAnother": "Tuto složku vlastní {{name}}.",
  "title.changeFolderNoOpHelp":
    "Nic nebude odesláno. Vyberte jinou složku nebo toto okno zavřete.",
  "title.changeFolderTrackedMediaCurrent":
    "Sledovaná média v aktuální složce: {{count}}",
  "title.changeFolderTrackedMediaSelected":
    "Sledovaná média ve vybrané složce: {{count}}",
  "title.changeFolderNoFilesMoved":
    "Žádné soubory nebudou přesunuty. Změní se pouze shoda složky a z ní odvozené propojení s médii.",
  "title.changeFolderConflictHeading": "Tato složka již patří jinému titulu",
  "title.changeFolderChooseResolution":
    "Pro pokračování zvolte Prohodit složky nebo Převzít složku.",
  "title.changeFolderSwapAction": "Prohodit složky",
  "title.changeFolderSwapExplanation":
    "{{owner}} převezme aktuální složku tohoto titulu a tento titul převezme {{folder}}. Oba tituly budou znovu zkontrolovány.",
  "title.changeFolderTakeOverAction": "Převzít složku",
  "title.changeFolderTakeOverExplanation":
    "Tento titul převezme {{folder}}. {{owner}} zůstane bez složky, stane se nepřiřazeným a bude potřebovat opravu v sekci Čekající importy.",
  "title.changeFolderConfirm": "Změnit složku",
  "title.changeFolderApplying": "Mění se složka…",
  "title.changeFolderApplyFailed": "Změna shody složky se nezdařila.",
  "title.changeFolderOutcomeAssigned": "{{name}} nyní vlastní {{folder}}.",
  "title.changeFolderOutcomeSwapped":
    "{{name}} a {{other}} si vyměnili složky.",
  "title.changeFolderOutcomeAlreadyOwned":
    "{{name}} tuto složku již vlastnil; nic se nezměnilo.",
  "title.changeFolderDetachedMedia": "Odpojených propojení s médii: {{count}}.",
  "title.changeFolderScanSummary":
    "Kontrola: zkontrolováno {{scanned}} • nalezena shoda {{matched}} • importováno {{imported}} • bez shody {{unmatched}}",
  "title.changeFolderDisplacedNote":
    "{{name}} přišel o {{folder}} a nyní je nepřiřazený. Opravte ho v sekci Čekající importy, kde se zobrazuje s důvodem „Vlastnictví složky změnil uživatel“.",
  "pendingImports.title": "Čekající importy",
  "pendingImports.description":
    "Vyřešte nepřiřazené položky na disku pro {{facet}} výběrem správné shody metadat.",
  "pendingImports.unresolvedCountOne": "1 nevyřešená",
  "pendingImports.unresolvedCountOther": "Nevyřešených: {{count}}",
  "pendingImports.loading": "Načítání čekajících importů…",
  "pendingImports.loadFailed": "Načtení čekajících importů se nezdařilo.",
  "pendingImports.empty": "Pro tuto oblast nejsou žádné čekající importy.",
  "pendingImports.path": "Cesta",
  "pendingImports.searchPlaceholder": "Hledat tituly metadat",
  "pendingImports.searchYearLabel": "Rok",
  "pendingImports.searchYearPlaceholder": "Rok",
  "pendingImports.searchYearClear": "Vymazat rok",
  "pendingImports.searchYearHint":
    "Rok pochází z kontroly. Vymažte ho pro rozšíření hledání, pokud je rok v názvu souboru chybný.",
  "pendingImports.searching": "Vyhledávání metadat…",
  "pendingImports.searchFailed": "Vyhledání metadat se nezdařilo.",
  "pendingImports.searchRequestFailed":
    "Vyhledávání metadat selhalo: {{error}}",
  "pendingImports.noSearchResults": "Nebyly nalezeny žádné shody metadat.",
  "pendingImports.searchAction": "Vyhledat a přiřadit",
  "pendingImports.match": "Shoda",
  "pendingImports.alreadyInLibrary": "Již v této knihovně",
  "pendingImports.attachToExistingTitle": "Připojit k existujícímu titulu",
  "pendingImports.attachSuccess": "Čekající import připojen k {{name}}.",
  "pendingImports.ignoredSection": "Ignorované importy",
  "pendingImports.ignore": "Ignorovat",
  "pendingImports.ignoreConfirmTitle": "Ignorovat import?",
  "pendingImports.ignoreSuccess": "Import pro {{name}} byl ignorován.",
  "pendingImports.ignoreFailed": "Ignorování čekajícího importu se nezdařilo.",
  "pendingImports.resolveSuccess": "Čekající import pro {{name}} byl vyřešen.",
  "pendingImports.resolveFailed": "Vyřešení čekajícího importu se nezdařilo.",
  "pendingImports.prev": "Předchozí",
  "pendingImports.next": "Další",
  "pendingImports.pageRange": "{{start}}–{{end}} / {{total}}",
  "pendingImports.library": "Knihovna:",
  "pendingImports.knownTitle": "Známý titul:",
  "pendingImports.reason.invalid_title_folder_ownership":
    "Složka titulu vyžaduje obnovu",
  "pendingImports.invalidOwnershipHelp":
    "Tento titul si nesprávně nárokuje kořenovou složku knihovny nebo její nadřazenou složku. Zkontrolujte obnovu složky titulu v sekci Aktivita. Původní soubory jsou zachovány; nepřesouvejte soubory do zaznamenaného kořene.",
  "pendingImports.reason.title_already_owns_another_folder":
    "Titul již vlastní jinou složku",
  "pendingImports.reason.metadata_id_lookup_unresolved":
    "Vyhledání metadat se nepodařilo spárovat s externím ID této složky; příští kontrola to zkusí znovu",
  "pendingImports.reason.folder_ownership_changed_by_user":
    "Vlastnictví složky změnil uživatel",
  "pendingImports.folderOwnershipChangedHelp":
    "Tato složka byla přidělena jinému titulu, takže tento titul už žádnou nemá. Otevřete titul a pomocí Změnit složku ho nasměrujte na správnou složku, nebo položku skryjte pomocí Ignorovat.",
  "pendingImports.ownershipConflictHelp":
    "Tento titul je přiřazen k {{folder}}. Přesuňte soubory této složky tam, nebo duplicitní složku smažte a poté spusťte kontrolu znovu.",
  "pendingImports.bindEpisodes": "Přiřadit epizody",
  "pendingImports.bindPreviewLoadFailed":
    "Načtení náhledu přiřazení se nezdařilo",
  "pendingImports.bindFailed": "Přiřazení čekajícího importu se nezdařilo.",
  "pendingImports.selectAtLeastOneEpisode": "Vyberte alespoň jednu epizodu.",
  "pendingImports.loadingEpisodeBindings": "Načítání přiřazení epizod...",
  "pendingImports.parsedHints": "Rozpoznané nápovědy:",
  "pendingImports.parsedHintSeason": " řada {{season}}",
  "pendingImports.parsedHintEpisodes": " • epizody {{episodes}}",
  "pendingImports.parsedHintAbsolute": " • absolutní {{absolute}}",
  "pendingImports.useSuggested": "Použít navrhované",
  "pendingImports.specials": "Speciály",
  "pendingImports.seasonNumber": "Řada {{number}}",
  "pendingImports.selectSeason": "Vybrat řadu",
  "pendingImports.bindSelectedEpisodes": "Přiřadit vybrané epizody",
  "pendingImports.episodeNumberLabel": "Epizoda {{number}}",
  "tvdb.searchResults": "Výsledky vyhledávání metadat",
  "tvdb.searchPrompt": "Vyhledejte metadata zadáním titulu výše.",
  "tvdb.searchByTvdb": "Hledat metadata",
  "nzb.searchResultsFor": "Výsledky NZB pro {{name}}",
  "nzb.noResultsYet": "Zatím žádné shody vydání.",
  "nzb.currentFile": "Aktuální soubor",
  "nzb.searchFailed": "Vyhledání vydání se nezdařilo.",
  "nzb.queue": "Zařadit do fronty",
  "nzb.movieAvailabilityWarning": "Automatické vyhledávání čeká: {{reason}}",
  "nzb.queueAdditionalFile": "Zařadit do fronty jako další soubor",
  "nzb.noConsideredResults": "Žádné vydání neprošlo profilem kvality.",
  "nzb.showScoringLog": "Zobrazit protokol hodnocení",
  "nzb.hideScoringLog": "Skrýt protokol hodnocení",
  "nzb.score": "Skóre",
  "nzb.recommended": "Doporučeno",
  "nzb.size": "Velikost",
  "nzb.scoringLog": "Protokol hodnocení",
  "nzb.total": "Celkem",
  "media.posterAlt": "Plakát k {{name}}",
  "activity.filterBarLabel": "Filtr aktivity stahování",
  "activity.import": "Import",
  "activity.activity": "Aktivita",
  "activity.history": "Historie",
  "activity.importFilter.importing": "Importuje se",
  "activity.importFilter.pending": "Čeká",
  "activity.importFilter.blocked": "Blokováno",
  "activity.importFilter.failed": "Selhalo",
  "activity.activityFilter.downloading": "Stahuje se",
  "activity.activityFilter.queued": "Ve frontě",
  "activity.activityFilter.paused": "Pozastaveno",
  "activity.activityFilter.postProcessing": "Zpracování po stažení",
  "activity.activityFilter.seeding": "Seeduje se",
  "activity.activityFilter.warning": "Varování",
  "activity.historyFilter.all": "Vše",
  "activity.historyFilter.success": "Úspěch",
  "activity.historyFilter.failed": "Selhalo",
  "activity.scryerSubmitted": "Odesláno do Scryeru",
  "activity.selectAllImportItems": "Vybrat všechny položky importu",
  "activity.selectImportItem": "Vybrat položku importu",
  "activity.selectedImportCount": "Vybráno: {{count}}",
  "activity.importEmpty": "Žádné položky importu.",
  "activity.activityEmpty": "Žádná aktivní stahování.",
  "activity.historyEmpty": "Žádné položky historie.",
  "activity.queueStale":
    "Data fronty jsou zastaralá; zobrazuje se poslední úspěšné obnovení.",
  "queue.title": "Titul",
  "queue.client": "Klient",
  "queue.status": "Stav",
  "queue.source": "Zdroj",
  "queue.progress": "Postup",
  "queue.size": "Velikost",
  "queue.state.queued": "Ve frontě",
  "queue.state.downloading": "Stahuje se",
  "queue.state.verifying": "Ověřuje se",
  "queue.state.repairing": "Opravuje se",
  "queue.state.extracting": "Rozbaluje se",
  "queue.state.postProcessing": "Zpracování po stažení",
  "queue.state.paused": "Pozastaveno",
  "queue.state.completed": "Dokončeno",
  "queue.state.importedSeeding": "Importováno · Seeduje se",
  "queue.state.importing": "Importuje se",
  "queue.state.importPending": "Import čeká",
  "queue.state.waitingForDiskSpace": "Čeká se na místo na disku",
  "activity.refreshTimedOut": "Obnovení vypršelo",
  "activity.refreshBehind":
    "Obnovení fronty nedosáhlo nejnovější aktualizace. Zkuste to znovu.",
  "activity.retry": "Zkusit znovu",
  "activity.loadMore": "Načíst další",
  "activity.staleData":
    "Zobrazuje se poslední úspěšný výsledek. Obnovení selhalo.",
  "activity.countUnknown":
    "Počet importů vyžadujících pozornost zatím není k dispozici",
  "activity.countStale":
    "Počet importů vyžadujících pozornost může být zastaralý",
  "queue.state.importBlocked": "Import blokován",
  "queue.state.warning": "Varování",
  "queue.state.failed": "Selhalo",
  "queue.state.ignored": "Ignorováno",
  "queue.state.unknown": "Neznámý",
  "queue.manualImport": "Ruční import",
  "queue.manualImporting": "Zařazování do fronty…",
  "queue.transfer.copying": "Kopírování",
  "queue.transfer.extracting": "Rozbalování",
  "queue.transfer.finalizing": "Dokončování",
  "queue.manualImportQueued": "Ruční import zařazen do fronty.",
  "queue.manualImportFailed": "Ruční import se nezdařil.",
  "queue.manualImportMultipleEpisodes": "Více epizod v tomto souboru",
  "queue.manualImportMultipleEpisodesLabel":
    "Přiřadit tomuto souboru více epizod",
  "queue.manualImportApplySuggestions": "Použít návrhy na vše",
  "queue.removeFailed": "Odebrání se nezdařilo",
  "queue.manualImportTooltip":
    "Ruční import je dostupný pro položky fronty vyžadující pozornost.",
  "queue.releaseTitle": "Vydání",
  "queue.blockReason": "Důvod blokace",
  "queue.blockReasonFallbackUnassigned":
    "Automatický import nedokázal určit titul v knihovně. Pro pokračování přiřaďte titul.",
  "queue.blockReasonFallbackEpisodic":
    "Automatický import nedokázal jednoznačně určit řadu a epizodu. Otevřete Ruční import a přiřaďte správnou řadu a epizodu.",
  "queue.blockReasonFallbackReview":
    "Automatický import vyžaduje kontrolu obsluhy. Otevřete Ruční import a potvrďte přiřazení souboru pro pokračování.",
  "queue.seeding.stateSeeding": "Seeduje se",
  "queue.seeding.stateGoalMet": "Cíl seedování splněn",
  "queue.seeding.stateHeldPrivate": "Seeduje se · soukromý",
  "queue.seeding.stateNeverRemove": "Seeduje se natrvalo",
  "queue.seeding.ratio": "Poměr {{value}}",
  "queue.seeding.seedTime": "Seedováno {{value}}",
  "queue.seeding.private": "Soukromý tracker",
  "queue.seeding.privateTooltip":
    "Soukromý tracker. Scryer tento torrent sám od sebe z klienta neodebere, pokud není splněn cíl profilu seedování.",
  "queue.errorCode": "Chybový kód",
  "queue.showDetails": "Zobrazit podrobnosti",
  "queue.hideDetails": "Skrýt podrobnosti",
  "queue.assignTitle": "Přiřadit titul",
  "queue.reassignTitle": "Změnit titul",
  "queue.assignTitleTitle": "Přiřadit titul ke stažení",
  "queue.assignTitleDescription":
    "Zvolte titul v knihovně, ke kterému toto blokované stažení patří. Scryer po přiřazení znovu zkontroluje import.",
  "queue.assignTitlePlaceholder": "Hledat tituly...",
  "queue.assignTitleResults": "Odpovídající tituly",
  "queue.assignTitleEmpty": "Nebyly nalezeny žádné odpovídající tituly.",
  "queue.assignTitleSearchHint": "Zadejte název titulu pro vyhledání…",
  "queue.assignTitleQueued": "Titul přiřazen. Import bude znovu vyhodnocen.",
  "queue.assignTitleFailed": "Přiřazení titulu se nezdařilo.",
  "queue.assignTitleBeforeImport":
    "Před spuštěním ručního importu přiřaďte titul.",
  "queue.ignore": "Ignorovat",
  "queue.ignoreSuccess": "Stažení ignorováno.",
  "queue.ignoreFailed": "Ignorování stažení se nezdařilo.",
  "queue.releaseHeldSources": "Uvolnit zadržené zdroje",
  "queue.releaseHeldSourcesConfirmTitle": "Uvolnit zadržené zdroje?",
  "queue.releaseHeldSourcesConfirmDescription":
    "{{download}} v klientovi {{client}} dokončil import, ale jeho zdroje jsou stále zadržené. Uvolnění zruší zadržení u každého importu tohoto stažení. Dočasné extrakční složky se odstraní pouze tehdy, pokud byl importován každý videosoubor v nich a nic jiného je nepoužívá.",
  "queue.releaseHeldSourcesClientRemoves":
    "{{client}} je nastaven na odebírání dokončených stažení. Jakmile Scryer potvrdí import, toto stažení a jeho soubory budou z klienta pro stahování odebrány.",
  "queue.releaseHeldSourcesClientKeeps":
    "{{client}} je nastaven na ponechání dokončených stažení, takže toto stažení zůstane v klientovi pro stahování.",
  "queue.releaseHeldSourcesSuccess": "Zadržené zdroje byly uvolněny.",
  "queue.releaseHeldSourcesFailed": "Uvolnění zadržených zdrojů se nezdařilo.",
  "queue.releaseHeldSourcesReasonSubtitlesPending":
    "Zdroje jsou zadrženy kvůli titulkům, které stále vyžadují pozornost. Uvolnění přestane na ně čekat a odstraní varování.",
  "queue.releaseHeldSourcesReasonSourceCleanupIncomplete":
    "Zdroje jsou zadrženy, protože odstranění souborů nahrazených tímto importem se nedokončilo.",
  "queue.releaseHeldSourcesReasonArchiveExtractionFailed":
    "Rozbalení archivu u tohoto stažení selhalo a jeho nerozbalené archivy nikdy nebyly importovány. Uvolnění označí stažení jako importované, což klientovi pro stahování umožní ho odebrat, ale pouze pokud je prokázáno, že byl importován každý soubor, který mělo obsahovat; vše, co nebylo nikdy importováno, bude odstraněno spolu s ním.",
  "queue.releaseHeldSourcesReasonUnknown":
    "Scryer nemá záznam o důvodu zadržení těchto zdrojů. Uvolnění označí stažení jako importované, což klientovi pro stahování umožní ho odebrat, ale pouze pokud je prokázáno, že byl importován každý soubor, který mělo obsahovat; vše v něm, co nebylo nikdy importováno, bude odstraněno spolu s ním.",
  "queue.releaseHeldSourcesTitles": "Toto vydání zahrnuje: {{titles}}",
  "queue.releaseHeldSourcesClientRemovesAfterSeeding":
    "{{client}} odebírá dokončená stažení, jakmile jsou splněny jejich požadavky na seedování. Jakmile Scryer potvrdí import, toto stažení a jeho soubory budou odebrány po dokončení seedování.",
  "queue.releaseHeldSourcesClientPolicyUnknown":
    "Scryer nedokázal určit, co {{client}} udělá s tímto stažením po uvolnění.",
  "queue.releaseHeldSourcesAwaitingImport":
    "Zadržené zdroje byly uvolněny. Stažení není plně importováno, takže se vrátilo k importu a nic nebylo odstraněno.",
  "queue.releaseHeldSourcesNotSettled":
    "Zadržené zdroje byly uvolněny, ale stažení se nepodařilo aktualizovat. Pro dokončení uvolněte znovu.",
  "queue.releaseHeldSourcesUnproven":
    "Zadržené zdroje byly uvolněny, ale nic neprokazuje, že byl importován každý soubor stažení, takže nebylo označeno jako importované a nic nebylo vyčištěno. Zbytek importujte nebo jej odstraňte sami.",
  "queue.releaseHeldSourcesNotSettledNothingCleaned":
    "Zadržené zdroje byly uvolněny, ale stažení nebylo vyřešeno a nic nebylo vyčištěno.",
  "queue.releaseHeldSourcesWorkspacesPreserved":
    "Některé dočasné extrakční složky byly zachovány: {{reasons}}.",
  "queue.releaseHeldSourcesPreservedNotOwned":
    "vlastnictví se nepodařilo potvrdit",
  "queue.releaseHeldSourcesPreservedUnsafe": "neočekávaný nebo nečitelný obsah",
  "queue.releaseHeldSourcesPreservedHoldsUnimportedVideo":
    "obsahuje video, které nikdy nebylo importováno",
  "queue.releaseHeldSourcesPreservedInUse": "stále potřebné pro jiný import",
  "queue.releaseHeldSourcesPreservedUnverified":
    "importované soubory se nepodařilo potvrdit",
  "queue.releaseHeldSourcesPreservedRemovalFailed": "odstranění se nezdařilo",
  "queue.releaseHeldSourcesPreservedDownloadNotImported":
    "nebylo prokázáno, že stažení bylo importováno",
  "queue.releaseHeldSourcesPreservedLookupIncomplete":
    "ne každou složku se podařilo zkontrolovat",
  "queue.markFailedSearchAgain": "Zablokovat, smazat data a vyhledat znovu",
  "queue.markFailedOnly": "Zablokovat a smazat data",
  "queue.markFailedSearchSuccess":
    "Zablokování a odstranění dat vyžádáno; nové získání povoleno.",
  "queue.markFailedOnlySuccess":
    "Zablokování a odstranění dat vyžádáno bez nového získání.",
  "queue.markFailedFailed": "Označení stažení jako selhané se nezdařilo.",
  "queue.bulkIgnoreSuccess": "Ignorováno stažení: {{count}}.",
  "queue.bulkIgnorePartial":
    "Ignorováno stažení: {{count}}; selhalo: {{failed}}.",
  "queue.bulkIgnoreFailed": "Ignorování vybraných stažení se nezdařilo.",
  "queue.pause": "Pozastavit",
  "queue.pauseSuccess": "Stažení pozastaveno.",
  "queue.pauseFailed": "Pozastavení stažení se nezdařilo.",
  "queue.resume": "Pokračovat",
  "queue.resumeSuccess": "Stažení pokračuje.",
  "queue.passwordRetryAccepted": "Heslo aktualizováno a opakování přijato.",
  "queue.passwordRetryRefused":
    "Klient pro stahování opakování odmítl. Stažení bylo zachováno.",
  "queue.passwordRetryUncertain":
    "Výsledek opakování je nejistý. Stažení je zachováno, dokud se čeká na sesouhlasení; další opakování nebylo odesláno.",
  "queue.passwordRetryFailed":
    "Opakování s heslem se nepodařilo spustit. Obnovte úlohu a zkontrolujte její aktuální stav.",
  "queue.resumeFailed": "Pokračování stažení se nezdařilo.",
  "queue.deleting": "Mazání…",
  "queue.deleteQueued": "Odebrání z klienta pro stahování zařazeno do fronty.",
  "queue.deleteFailed":
    "Odebrání stažení z klienta pro stahování se nezdařilo.",
  "queue.removeFromDownloader": "Odebrat z klienta pro stahování",
  "queue.deleteConfirmTitle": "Odebrat z klienta pro stahování",
  "queue.deleteConfirmDescription":
    "Odebrat toto stažení z klienta pro stahování? Po potvrzení odebrání klientem bude ignorováno.",
  "queue.bulkDeleteQueued":
    "Odebrání z klienta pro stahování zařazeno do fronty: {{count}} stažení.",
  "queue.bulkDeletePartial":
    "Odebrání zařazeno do fronty: {{count}} stažení; selhalo: {{failed}}.",
  "queue.bulkDeleteFailed": "Smazání vybraných stažení se nezdařilo.",
  "queue.bulkDeleteConfirmTitle": "Odebrat vybraná z klienta pro stahování",
  "queue.bulkDeleteConfirmDescription":
    "Odebrat vybraná stažení z klienta pro stahování: {{count}}? Po úspěšném odebrání budou ignorována.",
  "queue.manualUnavailableForResult":
    "Ruční fronta pro tento výsledek není dostupná.",
  "queue.belowMinimumSeeders":
    "Příliš málo seederů pro minimum tohoto indexeru.",
  "importHistory.retry": "Opakovat import",
  "importHistory.retryHelp":
    "Znovu vyhodnotit existující stažené soubory podle aktuálního titulu a profilu kvality.",
  "importHistory.retryWithPassword": "Opakovat s heslem",
  "importHistory.passwordRequired": "Vyžadováno heslo",
  "importHistory.passwordPlaceholder": "Heslo archivu",
  "importHistory.showPassword": "Zobrazit heslo",
  "importHistory.hidePassword": "Skrýt heslo",
  "importHistory.retrySuccess": "Opakování importu bylo spuštěno",
  "system.title": "Stav systému",
  "system.jobsTitle": "Úlohy",
  "system.auditTitle": "Audit",
  "system.auditLoaded": "Protokol auditu načten.",
  "system.auditEmpty": "Nebyly nalezeny žádné auditní události.",
  "system.auditTarget": "Cíl",
  "system.auditStream": "Stream",
  "system.notLoaded": "Stav systému nebyl načten.",
  "system.refreshing": "Obnovování",
  "system.loaded": "Stav systému načten.",
  "system.notReady": "Služba není připravena.",
  "system.serviceReady": "Služba připravena",
  "system.monitoredTitlesLabel": "Sledované tituly",
  "system.dbPathLabel": "Cesta k DB",
  "system.totalTitlesLabel": "Celkem titulů",
  "system.usersLabel": "Uživatelé",
  "system.facetLabel": "Oblasti",
  "system.recentEventSample": "Ukázka nedávných událostí",
  "system.sourcesTitle": "Zdroje dat",
  "system.sourcesSupport":
    "Pokud vám tyto služby pomáhají s vaší knihovnou, zvažte prosím jejich podporu darem, pokud můžete.",
  "system.sourceTvdbName": "TheTVDB",
  "system.sourceTmdbName": "The Movie Database (TMDB)",
  "system.sourceMalName": "MyAnimeList",
  "system.sourceAniBridgeName": "AniBridge",
  "settings.knownUsers": "Známí uživatelé",
  "settings.user": "Uživatel",
  "settings.createUser": "Vytvořit uživatele",
  "settings.username": "Uživatelské jméno",
  "settings.password": "Heslo",
  "settings.passwordMinLengthError":
    "Heslo je příliš krátké. Minimální počet znaků: {{min}}.",
  "settings.permissions": "Oprávnění",
  "settings.noUsers": "Nebyli načteni žádní uživatelé.",
  "settings.updatePassword": "Aktualizovat",
  "settings.temporaryPassword": "Dočasné heslo",
  "settings.resetPassword": "Obnovit heslo",
  "settings.setTemporaryPassword": "Nastavit dočasné heslo",
  "settings.temporaryPasswordHelp":
    "Uživatel musí toto heslo po místním přihlášení nahradit. Zašlete ho bezpečným kanálem mimo tento systém.",
  "settings.passwordChangeRequired": "Je vyžadována změna hesla",
  "settings.temporaryPasswordResetTitle": "Nastavit dočasné heslo?",
  "settings.temporaryPasswordResetDescription":
    "Tím se zruší aktivní relace uživatele {{name}} a po místním přihlášení si bude muset zvolit nové heslo.",
  "settings.apiKey": "Klíč API",
  "settings.existingIndexers": "Existující indexery",
  "settings.indexerProvider": "Poskytovatel",
  "settings.indexerInteractiveSearch": "Interaktivní",
  "settings.indexerAutoSearch": "Automatické",
  "settings.indexerStatus": "Stav",
  "settings.indexerDisabledUntil": "Vypnuto do {{time}}",
  "settings.indexerCoolingDownUntil": "Zpomaleno · obnoví se {{time}}",
  "settings.indexerCoolingDownHelp":
    "Indexer požádal Scryer o zpomalení. Vyhledávání se obnoví automaticky.",
  "settings.indexerLastError": "Chyba {{time}}",
  "settings.indexerLastSearched": "Hledáno {{time}}",
  "settings.indexerNoActivity": "Žádná aktivita",
  "settings.indexerSyncNow": "Synchronizovat nyní",
  "settings.indexerUpdate": "Aktualizovat indexer",
  "settings.indexerCreate": "Připojit indexer",
  "settings.indexerCreateNew": "Připojit indexer",
  "settings.indexerManagedParentHint":
    "Nadřazené konfigurace Prowlarr slouží pouze ke správě. Režimy vyhledávání pocházejí ze synchronizovaných podřízených indexerů.",
  "settings.indexerConfirmDiscardTitle": "Zahodit aktuální úpravy indexeru?",
  "settings.indexerConfirmDiscardDescription":
    "Aktuální úpravy indexeru budou ztraceny.",
  "settings.indexerFilterPlaceholder": "Filtrovat podle typu poskytovatele",
  "settings.noIndexersFound": "Nebyly nalezeny žádné indexery.",
  "settings.indexerConfig": "Konfigurace poskytovatele",
  "settings.indexerDynamicFieldsLoading": "Načítání polí pro tento výběr…",
  "settings.indexerDynamicFieldsError":
    "Nepodařilo se načíst pole pro tento výběr: {{error}}",
  "settings.indexerLoginChallengeLoading": "Načítání přihlašovacího obrázku…",
  "settings.indexerLoginChallengeImageAlt": "Přihlašovací obrázek",
  "settings.indexerLoginChallengeError":
    "Nepodařilo se načíst přihlašovací obrázek: {{error}}",
  "settings.indexerLoginChallengeRefresh": "Nový obrázek",
  "settings.indexerLoginChallengeRefusal.malformed":
    "Přihlašovací obrázek z pluginu se nepodařilo přečíst.",
  "settings.indexerLoginChallengeRefusal.kindUnsupported":
    "Tento indexer vyžaduje typ přihlašovací kontroly, který Scryer neumí zobrazit.",
  "settings.indexerLoginChallengeRefusal.imageTypeUnsupported":
    "Přihlašovací obrázek je ve formátu, který Scryer nezobrazuje.",
  "settings.indexerLoginChallengeRefusal.imageTooLarge":
    "Přihlašovací obrázek je příliš velký na zobrazení.",
  "settings.advancedConfig": "Pokročilé",
  "settings.managedIndexerBadge": "Spravováno",
  "settings.managedByIndexer": "Spravováno indexerem {{name}}",
  "settings.managedByParent": "Spravováno nadřazeným indexerem",
  "settings.managesIndexerCount": "Spravuje indexerů: {{count}}",
  "settings.managedIndexerReadOnly":
    "Spravované indexery jsou řízeny synchronizací s nadřazeným indexerem a nelze je upravovat přímo.",
  "settings.managedIndexerReadOnlyShort": "Pouze pro čtení",
  "settings.indexerRateLimits": "Omezení četnosti požadavků",
  "settings.indexerDownloadClient": "Klient pro stahování",
  "settings.indexerDownloadClientLabel": "Klient pro stahování pro {{name}}",
  "settings.indexerDownloadClientAutomatic": "Automaticky",
  "settings.indexerDownloadClientLoading": "Načítání klientů pro stahování…",
  "settings.indexerDownloadClientLoadRetry":
    "Znovu načíst klienty pro stahování",
  "settings.indexerDownloadClientCatalogStale":
    "Možnosti klienta pro stahování mohou být zastaralé.",
  "settings.indexerDownloadClientSelectionInvalid":
    "Zvolte Automaticky nebo klienta pro stahování kompatibilního s tímto indexerem.",
  "settings.indexerDownloadClientNotApplicable": "Nevztahuje se",
  "settings.indexerDownloadClientDisabled": "Vypnuto",
  "settings.indexerDownloadClientDisabledWarning":
    "{{name}} je vypnutý. Dokud nebude zapnutý, stahování selže.",
  "settings.indexerDownloadClientChooseAutomatic": "Zvolit Automaticky",
  "settings.indexerDownloadClientInvalidOption": "Neplatné: {{name}}",
  "settings.indexerDownloadClientInvalidMissing":
    "Přiřazený klient pro stahování chybí. Zvolte platného klienta nebo Automaticky.",
  "settings.indexerDownloadClientInvalidIncompatible":
    "Přiřazený klient pro stahování není s tímto indexerem kompatibilní.",
  "settings.indexerDownloadClientInvalidUnavailable":
    "Přiřazený klient pro stahování není dostupný. Zvolte platného klienta nebo Automaticky.",
  "settings.downloadClientDeleteConfirmDescription":
    "Smazat {{name}}? Indexery přiřazené k němu se vrátí na Automaticky.",
  "settings.newPassword": "Nové heslo",
  "settings.passwordManagedExternally": "Spravováno externě",
  "settings.mfa": "MFA",
  "settings.passkey": "Přístupový klíč",
  "settings.setUp": "Nastavit",
  "settings.notSetUp": "Nenastaveno",
  "settings.resetMfa": "Obnovit ověřovací faktory",
  "settings.resetMfaConfirm":
    "Obnovit ověřovací faktory pro {{name}}? Všechny aktivní relace a registrované ověřovací faktory budou zrušeny. Uživatel bude muset znovu zadat primární přihlašovací údaje. Pokud zásady přihlašování vyžadují MFA, bude muset zaregistrovat nový přístupový klíč nebo autentikátor; jinak se jeho příští přihlášení heslem podaří i bez registrace.",
  "settings.loginEnabled": "Přihlašování zapnuto",
  "settings.loginDisabled": "Přihlašování vypnuto",
  "settings.enableLogin": "Zapnout přihlašování",
  "settings.disableLogin": "Vypnout přihlašování",
  "settings.disableLoginConfirm":
    "Vypnout přihlašování pro {{name}}? Přihlašovací údaje a faktory zůstanou zachovány, ale stávající relace budou zrušeny.",
  "settings.rules": "Vlastní pravidla",
  "settings.noRulesFound": "Nejsou nastavena žádná vlastní pravidla.",
  "settings.trashPacksTitle": "Lokalizační balíčky TRaSH Guides",
  "settings.trashPacksHelp":
    "Balíčky odvozené z TRaSH Guides, které mění pouze skóre. Výběrem balíčku použijete jeho předdefinované zásady pro danou lokalitu.",
  "settings.trashPackFrenchConflict":
    "„{{name}}“ je již zapnutý. Před zapnutím dalšího francouzského balíčku ho vypněte.",
  "settings.ruleCreate": "Vytvořit pravidlo",
  "settings.ruleCreateNew": "Vytvořit nové pravidlo",
  "settings.ruleCopyAsCustom": "Zkopírovat jako vlastní pravidlo",
  "settings.ruleUpdate": "Aktualizovat pravidlo",
  "settings.ruleApplyTemplate": "Použít šablonu",
  "settings.ruleConfirmDiscardTitle": "Zahodit aktuální úpravy pravidla?",
  "settings.ruleConfirmDiscardDescription":
    "Aktuální úpravy pravidla budou ztraceny.",
  "settings.ruleDescription": "Popis",
  "settings.ruleRegoSource": "Zdrojový kód Rego",
  "settings.rulePriority": "Priorita",
  "settings.ruleAppliedFacets": "Platí pro oblasti",
  "settings.ruleAppliedFacetsHelp":
    "Ponechte prázdné pro použití na všechny typy médií.",
  "settings.ruleValidate": "Ověřit",
  "settings.ruleValidating": "Ověřování…",
  "settings.ruleValid": "Pravidlo je platné.",
  "settings.ruleValidationRequired": "Název a zdrojový kód Rego jsou povinné.",
  "settings.ruleLibrary": "Knihovna pravidel",
  "settings.ruleLibraryDescription":
    "Předpřipravená pravidla, která lze přidat jedním kliknutím. Výběrem šablony naplníte editor.",
  "settings.ruleLibraryAll": "Vše",
  "settings.ruleLibraryCommunityEmpty":
    "Nejsou dostupné žádné komunitní balíčky pravidel. Obnovte registr pluginů a zkontrolujte nové balíčky.",
  "settings.arrImportAction": "Importovat vlastní formát z Arr",
  "settings.ruleAiPrompt": "AI prompt",
  "settings.ruleAiPromptTitle": "Vytvořit pravidlo pomocí AI",
  "settings.ruleAiPromptDescription":
    "Popište požadovaný výsledek a poté prompt zkopírujte do AI asistenta.",
  "settings.ruleAiPromptOutcome": "Požadovaný výsledek",
  "settings.ruleAiPromptOutcomePlaceholder":
    "Například: Dej 100 bodů vydáním s japonským zvukem.",
  "settings.ruleAiPromptCopy": "Zkopírovat prompt",
  "settings.ruleAiPromptCopied": "Zkopírováno",
  "settings.arrImportTitle": "Import vlastních formátů z Arr",
  "settings.arrImportDescription":
    "Vložte export vlastního formátu ze Sonarru nebo Radarru a vytvořte z něj koncept nového pravidla Rego.",
  "settings.arrImportSource": "Zdrojová aplikace",
  "settings.arrImportJson": "JSON vlastního formátu",
  "settings.arrImportJsonPlaceholder":
    "Vložte jeden vlastní formát nebo pole vlastních formátů.",
  "settings.arrImportReviewAction": "Zkontrolovat formáty",
  "settings.arrImportReview": "Vlastních formátů k překladu: {{count}}",
  "settings.arrImportNoFormats":
    "V tomto JSON nebyly nalezeny žádné platné vlastní formáty.",
  "settings.arrImportInvalidJson":
    "Vložte platný JSON exportovaný ze Sonarru nebo Radarru.",
  "settings.arrImportReviewFailed":
    "Tyto vlastní formáty se nepodařilo zkontrolovat.",
  "settings.arrImportScore": "Skóre",
  "settings.arrImportScoreRequired":
    "Před překladem zadejte celočíselné skóre pro {{name}}.",
  "settings.arrImportSuggestedScore": "Navrhované skóre: {{score}}.",
  "settings.arrImportUseSuggestion": "Použít návrh",
  "settings.arrImportScoreReasonExported": "Používá skóre obsažené v exportu.",
  "settings.arrImportScoreReasonDefault":
    "Používá výchozí skóre z exportu; další volby profilu jsou dostupné níže.",
  "settings.arrImportScoreReasonPreference":
    "Název formátu vyjadřuje preferenci.",
  "settings.arrImportScoreReasonPenalty":
    "Název formátu označuje něco nežádoucího.",
  "settings.arrImportScoreReasonExtras":
    "Formát pravděpodobně označuje bonusy nebo ukázky.",
  "settings.arrImportScoreSuggestionHelp":
    "Návrhy jsou upravitelným výchozím bodem. Skóre odpovídajících formátů se sčítá; penalizace lze vyvážit jinými skóre.",
  "settings.arrImportAmbiguousScore":
    "Tento formát má více navrhovaných skóre. Zvolte skóre, které chcete použít.",
  "settings.arrImportBlockScoreHelp":
    "Celkové skóre musí být vyšší než -9000 a splňovat minimum profilu. Povinné požadavky platí i nadále.",
  "settings.arrImportTranslate": "Přeložit do Rego",
  "settings.arrImportTranslationFailed":
    "Překlad se nezdařil. Vložený JSON je stále dostupný ke kontrole nebo opakování.",
  "settings.arrImportResultSummary":
    "Přeloženo: {{translated}}; vypnuto k ruční opravě: {{disabled}}.",
  "settings.arrImportDraftName": "Importované vlastní formáty z Arr",
  "settings.arrImportDraftDescription":
    "Importováno z JSON vlastního formátu Arr.",
  "settings.arrImportDiagnostics": "Poznámky k překladu",
  "settings.arrImportApply": "Použít importovaný koncept",
  "settings.refTitle": "Referenční přehled vstupního kontextu",
  "settings.refSubtitle":
    "Kliknutím rozbalíte celý referenční přehled polí dostupných vašim pravidlům Rego.",
  "settings.refIntro":
    "Každé pravidlo dostane vstupní objekt s pěti dokumenty na nejvyšší úrovni. Váš kód Rego k nim přistupuje přes input.release, input.profile, input.context, input.builtin_score a input.file. input.file je během hodnocení před stažením null a naplní se až během vyhodnocení po importu.",
  "settings.refColField": "Pole",
  "settings.refColType": "Typ",
  "settings.refColDescription": "Popis",
  "settings.refColFunction": "Funkce",
  "settings.refColReturns": "Návratová hodnota",
  "settings.refSectionRelease": "— Analyzovaná metadata vydání",
  "settings.refReleaseRawTitle":
    "Původní nerozparsovaný název vydání z indexeru",
  "settings.refReleaseQuality":
    "Zjištěná úroveň kvality (např. 2160P, 1080P, 720P)",
  "settings.refReleaseSource": "Zdroj média (např. WEB-DL, BLURAY, HDTV)",
  "settings.refReleaseVideoCodec": "Video kodek (např. H.265, H.264, AV1)",
  "settings.refReleaseAudio": "Hlavní zvukový kodek (např. DDP, AAC, TrueHD)",
  "settings.refReleaseAudioCodecs": "Všechny zjištěné zvukové kodeky jako pole",
  "settings.refReleaseGuideFacts": "Fakta z průvodce zjištěná z metadat vydání",
  "settings.refReleaseNormalizedTokens":
    "Seřazené, normalizované tokeny vytvořené parserem vydání",
  "settings.refReleaseAudioChannels":
    "Konfigurace zvukových kanálů (např. 5.1, 7.1, 2.0)",
  "settings.refReleaseLangsAudio":
    "Kódy jazyků zvuku ISO 639-3 (např. eng, jpn). Zahrnuje jazyky, které uvedl záznam indexeru, zmrazené v okamžiku stažení vydání; soubory naskenované, přejaté nebo importované před zachováním snímků záznamů přispívají jen tím, co je vidět z názvu a souboru",
  "settings.refReleaseIndexerLanguages":
    "Kódy jazyků zvuku ISO 639-3, které sám uvedl záznam indexeru, zmrazené v okamžiku stažení vydání. Nikdy neodvozeno z názvu; prázdné, pokud indexer nic neuvedl",
  "settings.refReleaseLangsSub": "Kódy jazyků titulků ISO 639-3",
  "settings.refReleaseIsDualAudio":
    "True, pokud bylo zjištěno více zvukových stop",
  "settings.refReleaseIsAtmos": "True, pokud je přítomen zvuk Dolby Atmos",
  "settings.refReleaseIsDV": "True, pokud jsou zjištěna metadata Dolby Vision",
  "settings.refReleaseHasHdrFallback":
    "True, pokud parser vydání zjistí záložní vrstvu HDR",
  "settings.refReleaseDetectedHdr": "True, pokud je zjištěno HDR (jiné než DV)",
  "settings.refReleaseIsRemux": "True pro plný remux (bez překódování)",
  "settings.refReleaseIsBdDisk": "True pro zdroj Blu-ray disk",
  "settings.refReleaseStereoscopy":
    "Prezentace deklarovaná názvem vydání; null znamená neznámé",
  "settings.refSectionStereoscopy": "— Stereoskopická prezentace",
  "settings.refStereoPresentation":
    "two_d, three_d nebo mixed_2d_3d; jde o tvrzení z názvu, nikoli ověřené vlastnosti streamu",
  "settings.refStereoLayout":
    "side_by_side, top_bottom, frame_sequential, row_interleaved, column_interleaved, checkerboard nebo anaglyph; null, pokud není uvedeno, je v rozporu, nebo smíšené",
  "settings.refStereoSampling":
    "half nebo full pro side-by-side nebo top/bottom; null, pokud není známo",
  "settings.refStereoEncoding":
    "mvc, pokud je výslovně potvrzeno značkou 3D; null, pokud není známo",
  "settings.refReleaseStereoscopyTokens":
    "Kanonické stereo tokeny: stereo:two_d, stereo:three_d, stereo:mixed_2d_3d, stereo:has_3d, stereo:layout:<layout>, stereo:sampling:half/full, stereo:encoding:mvc a stereo:<field>_conflict. Prázdné znamená neznámé. Surové normalized_tokens si zachovávají svůj původní význam.",
  "settings.refReleaseRevision":
    "Číselná revize vydání: 0 pro originál, 1 pro PROPER/REPACK/RERIP, 2 pro REPACK2 a tak dále",
  "settings.refReleaseIsProper": "True, pokud je vydání označeno jako Proper",
  "settings.refReleaseIsRepack": "True, pokud je vydání označeno jako Repack",
  "settings.refReleaseIsAiEnhanced":
    "True, pokud parser zjistil značku vylepšení pomocí AI",
  "settings.refReleaseIsHardcodedSubs":
    "True, pokud jsou v názvu vydání zjištěny vypálené titulky",
  "settings.refReleaseIsPasswordProtected":
    "True, pokud záznam indexeru označil vydání jako chráněné heslem. Fakt ze záznamu indexeru zmrazený v okamžiku stažení vydání; null, pokud to indexer neuvedl, a null pro soubory naskenované, přejaté nebo importované před zachováním snímků záznamů",
  "settings.refReleaseIsHdr10Plus":
    "True, pokud je z metadat vydání zjištěno HDR10+",
  "settings.refReleaseIsHlg": "True, pokud je z metadat vydání zjištěno HLG",
  "settings.refReleaseIs10Bit":
    "True, pokud je z metadat vydání zjištěno 10bitové video",
  "settings.refReleaseIsUncensored":
    "True, pokud je vydání označeno jako necenzurované",
  "settings.refReleaseIsDubsOnly":
    "True, pokud je vydání označeno jako pouze s dabingem",
  "settings.refReleaseHasReleaseGroup":
    "True, pokud byla rozpoznána neprázdná release group",
  "settings.refReleaseIsObfuscated":
    "True, pokud se zdá, že název vydání je zamaskovaný",
  "settings.refReleaseIsRetagged":
    "True, pokud se zdá, že název vydání byl přeznačen nebo znovu zveřejněn",
  "settings.refReleaseStreamingService":
    "Zjištěná značka streamovací služby, pokud je přítomna",
  "settings.refReleaseEdition":
    "Značka edice, jako extended, IMAX nebo criterion",
  "settings.refReleaseAnimeVersion":
    "Číslo verze anime, jako v2 nebo v3, pokud je přítomno",
  "settings.refReleaseEpisodeReleaseType":
    "Druh balení epizody, jako single_episode, multi_episode, season_pack nebo unknown",
  "settings.refReleaseIsSeasonPack":
    "True, pokud vydání zřejmě obsahuje úplný nebo částečný balíček řady",
  "settings.refReleaseIsMultiEpisode":
    "True, pokud vydání zřejmě obsahuje více epizod",
  "settings.refReleaseGroup": "Release group nebo jméno uploadera",
  "settings.refReleaseYear": "Rok vydání",
  "settings.refReleaseParseConf": "Skóre spolehlivosti analýzy (0,0 až 1,0)",
  "settings.refReleaseSizeBytes":
    "Velikost souboru v bajtech (pro převod použijte scryer.size_gib())",
  "settings.refReleaseAgeDays":
    "Celé dny od času zveřejnění u indexeru do okamžiku stažení vydání. Fakt ze záznamu indexeru zmrazený při stažení: stáří při stažení se dál nemění. Dosud nestažené vydání stárne nyní. Null, pokud není znám čas zveřejnění, a null pro soubory naskenované, přejaté nebo importované před zachováním snímků záznamů",
  "settings.refReleaseThumbsUp":
    "Počet kladných hlasů z indexeru. Fakt ze záznamu indexeru zmrazený v okamžiku stažení vydání; null, pokud indexer hlasy neuváděl, a null pro soubory naskenované, přejaté nebo importované před zachováním snímků záznamů",
  "settings.refReleaseThumbsDown":
    "Počet záporných hlasů z indexeru. Fakt ze záznamu indexeru zmrazený v okamžiku stažení vydání; null, pokud indexer hlasy neuváděl, a null pro soubory naskenované, přejaté nebo importované před zachováním snímků záznamů",
  "settings.refReleaseExtra":
    "Atributy záznamu specifické pro indexer (ploché hodnoty a seznamy hodnot), dostupné jako input.release.extra.<key>. Fakta ze záznamu indexeru zmrazená v okamžiku stažení vydání; u souborů naskenovaných, přejatých nebo importovaných před zachováním snímků záznamů jde o prázdný objekt, takže každé vyhledání input.release.extra.<key> je nedefinované (nikoli null) a porovnání jako == null nic nenajde",
  "settings.ruleTestModeLabel": "Co otestovat",
  "settings.ruleTestModeRelease": "Název vydání",
  "settings.ruleTestModeStoredFile": "Otestovat uložený soubor",
  "settings.ruleTestStoredFile": "Uložený soubor",
  "settings.ruleTestStoredFileHelp":
    "Hodnoceno s vlastní velikostí souboru a fakty ze záznamu indexeru zmrazenými v okamžiku stažení.",
  "settings.ruleTestStoredFileSelectTitle": "Nejprve vyberte titul z knihovny",
  "settings.ruleTestStoredFileLoading": "Načítání souborů…",
  "settings.ruleTestStoredFilePlaceholder": "Vyberte soubor",
  "settings.ruleTestStoredFileNone":
    "Pro tento titul nebo epizodu nejsou žádné soubory",
  "settings.ruleTestStoredFileFilter": "Filtrovat soubory",
  "settings.ruleTestStoredFileLoadFailed":
    "Soubory tohoto titulu se nepodařilo načíst.",
  "settings.ruleTestListingFacts": "Fakta ze záznamu",
  "settings.ruleTestListingFactsHelp":
    "Volitelná fakta ze záznamu indexeru. Vše ponechané prázdné je pro pravidla neznámé (null). Vydání stárne od času zveřejnění do teď.",
  "settings.ruleTestListingPublishedAt": "Zveřejněno",
  "settings.ruleTestListingPublishedAtPlaceholder":
    "2024-01-31, 2024-01-31T12:00:00Z, nebo Wed, 31 Jan 2024 12:00:00 +0000",
  "settings.ruleTestListingThumbsUp": "Kladné hlasy",
  "settings.ruleTestListingThumbsDown": "Záporné hlasy",
  "settings.ruleTestListingPasswordProtected": "Chráněno heslem",
  "settings.ruleTestListingUnknown": "Neznámé",
  "settings.ruleTestListingYes": "Ano",
  "settings.ruleTestListingNo": "Ne",
  "settings.ruleTestListingIndexerLanguages":
    "Jazyky indexeru (oddělené čárkou)",
  "settings.ruleTestListingExtra": "Další atributy (objekt JSON)",
  "settings.ruleTestListingPublishedAtInvalid":
    "Čas zveřejnění musí být datum (2024-01-31), čas RFC 3339 se sekundami a pásmem (2024-01-31T12:00:00Z), nebo datum RFC 2822 (Wed, 31 Jan 2024 12:00:00 +0000).",
  "settings.ruleTestListingVotesInvalid":
    "Hlasy musí být celá nezáporná čísla.",
  "settings.ruleTestListingExtraInvalid":
    "Další atributy musí být objekt JSON.",
  "settings.ruleTestResultRelease": "Ohodnocené vydání",
  "settings.ruleTestResultListingUnknown":
    "Fakta ze záznamu jsou neznámá: tento soubor byl naskenován, přejat nebo importován před zachováním snímků záznamů, takže pro pravidla je každý fakt ze záznamu null.",
  "settings.ruleTestResultAgeDays": "Stáří (dny)",
  "settings.ruleTestResultAgeAtGrab": "{{days}} při stažení",
  "settings.ruleTestResultCapturedAt": "Zachyceno",
  "settings.refSectionProfile": "— Konfigurace aktivního profilu kvality",
  "settings.refProfileId": "Unikátní identifikátor profilu",
  "settings.refProfileName": "Zobrazovaný název profilu",
  "settings.refProfileQualityTiers":
    "Seřazený seznam přijatelných úrovní kvality",
  "settings.refProfileArchivalQuality":
    "Nejvyšší úroveň kvality považovaná za archivní",
  "settings.refProfileAllowUnknown": "Zda jsou přijímány nerozpoznané kvality",
  "settings.refProfileSourceAllow":
    "Povolené zdroje (prázdné znamená všechny povolené)",
  "settings.refProfileSourceBlock": "Blokované zdroje",
  "settings.refProfileVCodecAllow":
    "Povolené video kodeky (prázdné znamená všechny povolené)",
  "settings.refProfileVCodecBlock": "Blokované video kodeky",
  "settings.refProfileACodecAllow":
    "Povolené zvukové kodeky (prázdné znamená všechny povolené)",
  "settings.refProfileACodecBlock": "Blokované zvukové kodeky",
  "settings.refProfileAtmosPreferred":
    "Profil preferuje Dolby Atmos, je-li dostupný",
  "settings.refProfileDVAllowed": "Vydání s Dolby Vision jsou přijímána",
  "settings.refProfileHdrAllowed": "Vydání s HDR jsou přijímána",
  "settings.refProfilePreferRemux": "Profil preferuje plná remux vydání",
  "settings.refProfileAllowBdDisk": "Zdroje Blu-ray disk jsou přijímány",
  "settings.refProfileAllowUpgrades": "Upgrade stávajících souborů je povolen",
  "settings.refProfilePreferDualAudio":
    "Profil preferuje vydání s duálním zvukem",
  "settings.refProfileRequiredLangs": "Povinné jazyky zvuku (kódy ISO 639-3)",
  "settings.refProfileScoringPersona":
    "Výsledná persona: balanced, audiophile, efficient nebo compatible",
  "settings.refProfileScoringOverrides":
    "Explicitní volby profilu; null ponechává výchozí hodnotu persony beze změny",
  "settings.refSectionScoringOverrides": "— Přepsání hodnocení",
  "settings.refOverrideX265":
    "Povolit HEVC jiné než 4K; null dědí výchozí hodnotu persony",
  "settings.refOverrideDolbyVision":
    "Odmítnout Dolby Vision bez záložní vrstvy; null dědí výchozí hodnotu persony",
  "settings.refOverrideCompact":
    "Preferovat kompaktní enkódování; null dědí výchozí hodnotu persony",
  "settings.refOverrideLossless":
    "Preferovat bezztrátový zvuk; null dědí výchozí hodnotu persony",
  "settings.refOverrideUpscaled":
    "Penalizovat upscalovaná vydání; null dědí výchozí hodnotu persony",
  "settings.refCtxCoverageTotalRuntime":
    "Celková stopáž pokrytí vydání v minutách, je-li známa",
  "settings.refCtxCoverageMemberRuntime":
    "Stopáž jedné pokryté položky v minutách, je-li známa",
  "settings.refCtxCoverageMemberCount": "Počet pokrytých položek, je-li znám",
  "settings.refSectionContext": "— Kontext titulu a vyhledávání",
  "settings.refCtxTitleId":
    "Interní id titulu Scryer, pokud je již znám konkrétní titul",
  "settings.refCtxLibraryName":
    "Název výsledné knihovny, pokud je vyhodnocení omezeno na konkrétní knihovnu",
  "settings.refCtxMediaType":
    "Vyhodnocovaná oblast média, typicky movie, series nebo anime",
  "settings.refCtxCategory":
    "Nápověda kategorie použitá pro hodnocení a vyhodnocení pravidel",
  "settings.refCtxOriginalLanguage":
    "Normalizovaný jazyk originálního názvu, pokud ho metadata poskytují",
  "settings.refCtxOriginalCountry":
    "Normalizovaná země originálního názvu, pokud ji metadata poskytují",
  "settings.refCtxInferredOriginalAudioLanguage":
    "Normalizovaný originální jazyk zvuku odvozený pro neoznačená vydání",
  "settings.refCtxTags":
    "Uživatelem definované štítky titulu, malými písmeny, z registru štítků, který spravuje administrátor v Nastavení; vyhrazené položky scryer: jsou vyloučeny",
  "settings.refCtxHasExisting":
    "True, pokud pro tento titul již existuje soubor",
  "settings.refCtxExistingScore":
    "Vestavěné skóre existujícího souboru (pokud existuje)",
  "settings.refCtxSearchMode":
    "Režim vyhodnocení. Aktuální hodnoty jsou auto a post_download",
  "settings.refCtxRuntimeMin": "Stopáž obsahu v minutách",
  "settings.refCtxIsAnime": "True, pokud je titul kategorizován jako anime",
  "settings.refCtxIsFiller": "True, pokud je epizoda označena jako výplňová",
  "settings.refSectionBuiltinScore":
    "— Výsledky vestavěného hodnoticího enginu",
  "settings.refBuiltinTotal": "Celkové skóre od vestavěného hodnoticího enginu",
  "settings.refBuiltinBlocked":
    "True, pokud vestavěná pravidla toto vydání zablokovala",
  "settings.refBuiltinCodes":
    "Seznam použitých hodnoticích kódů (např. quality_tier_0)",
  "settings.refSectionFile":
    "— Skutečná analyzovaná metadata souboru (null před stažením)",
  "settings.refFileVideoCodec": "Ověřený video kodek z analýzy média",
  "settings.refFileVideoWidth": "Ověřená šířka videa v pixelech",
  "settings.refFileVideoHeight": "Ověřená výška videa v pixelech",
  "settings.refFileVideoBitrateKbps": "Ověřený datový tok videa v kb/s",
  "settings.refFileVideoBitDepth": "Ověřená bitová hloubka videa",
  "settings.refFileVideoHdrFormat":
    "Ověřený formát HDR, jako Dolby Vision, HDR10, HDR10+ nebo HLG",
  "settings.refFileDoviProfile": "Číslo profilu Dolby Vision, je-li přítomno",
  "settings.refFileDoviBlCompatId":
    "Id kompatibility základní vrstvy Dolby Vision, je-li přítomno",
  "settings.refFileVideoFrameRate":
    "Ověřený řetězec snímkové frekvence z analýzy média",
  "settings.refFileVideoProfile":
    "Ověřený profil kodeku, jako Main 10 nebo High",
  "settings.refFileAudioCodec": "Hlavní ověřený zvukový kodek",
  "settings.refFileAudioProfile":
    "Hlavní ověřený zvukový profil nebo rozšiřující popisek, jako LC, Dolby TrueHD + Dolby Atmos nebo DTS-HD MA + DTS:X IMAX",
  "settings.refFileAudioChannels": "Počet kanálů hlavní ověřené zvukové stopy",
  "settings.refFileAudioBitrateKbps":
    "Datový tok hlavní ověřené zvukové stopy v kb/s",
  "settings.refFileAudioLanguages":
    "Všechny zjištěné jazyky zvuku ze skutečného souboru",
  "settings.refFileAudioStreams":
    "Podrobnosti o jednotlivých zvukových stopách; viz input.file.audio_streams[] níže",
  "settings.refFileSubtitleLanguages":
    "Všechny zjištěné jazyky titulků ze skutečného souboru",
  "settings.refFileSubtitleCodecs":
    "Všechny zjištěné kodeky titulků ze skutečného souboru",
  "settings.refFileSubtitleStreams":
    "Podrobnosti o jednotlivých stopách titulků; viz input.file.subtitle_streams[] níže",
  "settings.refFileHasMultiAudio":
    "True, pokud je ve skutečném souboru přítomno více zvukových stop",
  "settings.refFileDurationSeconds": "Ověřená stopáž v sekundách",
  "settings.refFileNumChapters":
    "Ověřený počet kapitol, pokud kontejner kapitoly obsahuje",
  "settings.refFileContainerFormat":
    "Ověřený formát kontejneru, jako matroska nebo mp4",
  "settings.refSectionAudioStreams": "— Vnořené položky zvukových stop",
  "settings.refAudioStreamCodec": "Zvukový kodek jednotlivé stopy",
  "settings.refAudioStreamProfile":
    "Ověřený zvukový profil nebo rozšiřující popisek jednotlivé stopy",
  "settings.refAudioStreamChannels": "Počet kanálů jednotlivé stopy",
  "settings.refAudioStreamLanguage": "Kód jazyka jednotlivé stopy",
  "settings.refAudioStreamBitrateKbps": "Datový tok jednotlivé stopy v kb/s",
  "settings.refSectionSubtitleStreams": "— Vnořené položky stop titulků",
  "settings.refSubtitleStreamCodec": "Kodek titulků jednotlivé stopy",
  "settings.refSubtitleStreamLanguage": "Kód jazyka jednotlivé stopy",
  "settings.refSubtitleStreamName":
    "Název stopy titulků jednotlivé stopy, je-li dostupný",
  "settings.refSubtitleStreamForced":
    "True, pokud je stopa titulků označena jako vynucená",
  "settings.refSubtitleStreamDefault":
    "True, pokud je stopa titulků označena jako výchozí",
  "settings.refSectionBuiltins": "Vlastní funkce Scryer",
  "settings.refBuiltinsIntro":
    "Tyto pomocné funkce jsou ve vašich pravidlech Rego dostupné navíc ke všem standardním vestavěným funkcím OPA.",
  "settings.refFnBlockScore":
    "Vrátí -10000 jako penalizaci, kterou mohou vyvážit jiná skóre",
  "settings.refFnSizeGib":
    "Převede bajty na GiB (např. scryer.size_gib(input.release.size_bytes) > 50)",
  "settings.refFnLangMatches":
    'Porovná kódy ISO 639-3 s aliasy (např. scryer.lang_matches("jpn", "ja") je true)',
  "settings.refFnNormalizeSource":
    "Normalizuje názvy zdrojů (např. „webdl“ se stane „WEB-DL“)",
  "settings.refFnNormalizeCodec":
    "Normalizuje názvy kodeků (např. „h264“ se stane „H.264“)",
  "settings.refSectionSandbox": "Sandbox a omezení",
  "settings.refSandboxIntro":
    "Všechna uživatelská pravidla běží uvnitř izolovaného vyhodnocovače Rego s těmito omezeními:",
  "settings.refSandboxNoIO":
    "Žádný přístup k síti ani souborovému systému — pravidla nemohou odesílat HTTP požadavky, číst soubory ani provádět žádné I/O operace. Standardní vestavěné funkce OPA jako http.send nejsou dostupné.",
  "settings.refSandboxPkgIsolation":
    "Izolace balíčků — každé pravidlo je automaticky umístěno do vlastního jmenného prostoru balíčku. Pravidla nemohou číst ani měnit data jiných pravidel.",
  "settings.refSandboxReadOnly":
    "Vstup pouze pro čtení — pravidla mohou pouze číst vstupní dokument. Nemohou měnit stav aplikace ani uchovávat data mezi vyhodnoceními.",
  "settings.refSandboxOutputRestricted":
    "Výstup omezen na score_entry — z každého pravidla se extrahuje pouze mapa score_entry. Všechny ostatní definice nebo přiřazení jsou ignorovány.",
  "settings.refSandboxIntegerOnly":
    "Jen celočíselné změny skóre — hodnoty v score_entry musí být celá čísla v rozsahu i32. Desetinná čísla jsou při ověřování odmítnuta (pro převod použijte round() nebo ceil()).",
  "settings.refSandboxValidation":
    "Ověření před uložením — pravidla jsou před uložením zkompilována a zkušebně spuštěna na syntetickém vstupu. Neplatná pravidla jsou odmítnuta.",
  "settings.refSandboxErrorIsolation":
    "Izolace chyb — pokud jedno pravidlo za běhu selže, ostatní pravidla se dál vyhodnocují normálně. Chyby se zaznamenávají do protokolu, ale nikdy nezastaví hodnocení.",
  "settings.refSectionOutput": "Formát výstupu pravidla",
  "settings.refOutputIntro":
    "Pravidla musí definovat pravidlo score_entry, které vrací mapu textových klíčů (kódy skóre) na celočíselné hodnoty (bodové změny). Kladné hodnoty vydání zvýhodňují, záporné penalizují a scryer.block_score() přispívá vratnou penalizací -10000. O způsobilosti se rozhoduje až po zohlednění všech příspěvků; povinné požadavky profilu nadále blokují. Pravidla založená na souboru před stažením přirozeně nic nedělají, protože input.file je až do vyhodnocení po stažení null.",
  "settings.subtitles": "Titulky",
  "settings.subtitlesDescription":
    "Nastavte jazyky titulků, prahy shody, chování synchronizace a připojení k poskytovatelům.",
  "settings.existingSubtitleProviders": "Nastavení poskytovatelé titulků",
  "settings.subtitleProviderType": "Poskytovatel",
  "settings.subtitleProviderStatus": "Stav",
  "settings.subtitleProviderFacets": "Typy obsahu",
  "settings.subtitleProviderFacetsHelp":
    "Scryer prohledává tohoto poskytovatele jen pro vybrané typy obsahu. Doporučení pluginu nastaví tyto výchozí hodnoty, ale neomezují vaši volbu.",
  "settings.subtitleProviderEmpty":
    "Zatím nejsou nastaveni žádní poskytovatelé titulků.",
  "settings.subtitleProviderCreate": "Přidat poskytovatele titulků",
  "settings.subtitleProviderCreateNew": "Vytvořit nového poskytovatele titulků",
  "settings.subtitleProviderEdit": "Upravit poskytovatele titulků",
  "settings.subtitleProviderConfirmDiscardTitle":
    "Zahodit aktuální úpravy poskytovatele titulků?",
  "settings.subtitleProviderConfirmDiscardDescription":
    "Aktuální úpravy poskytovatele titulků budou ztraceny.",
  "settings.subtitleProviderNamePlaceholder": "OpenSubtitles",
  "settings.subtitleProviderCreated": "Poskytovatel titulků vytvořen.",
  "settings.subtitleProviderUpdated": "Poskytovatel titulků aktualizován.",
  "settings.subtitleProviderDeleted": "{{name}} smazán.",
  "settings.subtitleProviderNoActivity": "Zatím žádná aktivita",
  "settings.subtitleProviderDisabledUntil": "Vypnuto do {{time}}",
  "settings.subtitleProviderLastError": "Poslední chyba {{time}}",
  "settings.subtitleProviderUnknownType":
    "Tento typ poskytovatele je nainstalován, ale jeho schéma polí zatím není dostupné.",
  "settings.subtitleProviderSecretStored":
    "Uložená hodnota zůstane zachována, dokud ji nenahradíte",
  "settings.sub.languages": "Jazyky titulků",
  "settings.sub.languagePickerAriaLabel": "Vybrat jazyky titulků",
  "settings.sub.languagePickerSelect": "Vyberte jazyky...",
  "settings.sub.languagePickerSearch": "Hledat jazyky...",
  "settings.sub.languagePickerEmpty": "Nebyly nalezeny žádné jazyky",
  "settings.sub.languagePickerRemove": "Odebrat {{language}}",
  "settings.sub.hiPreference": "Pro neslyšící",
  "settings.sub.forcedOnly": "Pouze vynucené titulky",
  "settings.sub.minScoreSeries": "Minimální procento shody pro seriály",
  "settings.sub.minScoreMovie": "Minimální procento shody pro filmy",
  "settings.sub.minScoreHelp":
    "Titulky pod tímto procentem shody se přeskočí. Seriály potřebují vyšší práh, protože shoda na úrovni epizody musí být přesná.",
  "settings.sub.searchInterval": "Interval hledání (hodiny)",
  "settings.sub.autoDownload": "Automaticky stáhnout při importu",
  "settings.sub.excludeAi": "Vyloučit titulky přeložené AI",
  "settings.sub.excludeMachine": "Vyloučit strojově přeložené titulky",
  "settings.sub.syncEnabled": "Automaticky opravit časování titulků",
  "settings.sub.syncEnabledHelp":
    "Pomocí analýzy zvuku opravuje posun v časování stažených titulků.",
  "settings.sub.syncThresholdSeries":
    "Automaticky opravit při tomto procentu nebo nižším (seriály)",
  "settings.sub.syncThresholdMovie":
    "Automaticky opravit při tomto procentu nebo nižším (filmy)",
  "settings.sub.syncThresholdHelp":
    "Titulky s nízkým skóre mívají častěji problémy s časováním. Titulky nad tímto procentem shody se považují za dobře seřízené.",
  "settings.sub.syncMaxOffset": "Maximální oprava (sekundy)",
  "settings.sub.syncMaxOffsetHelp":
    "Přeskočí změny časování větší než tato hodnota, aby se nepoškodily titulky, které jsou pravděpodobně již dost přesné.",
  "settings.sub.syncPluginRequiredTitle":
    "Vyžadována rozšířená synchronizace titulků",
  "settings.sub.syncPluginRequiredDescription":
    "Nainstalujte {{plugin}} a odemkněte nastavení automatické opravy časování.",
  "settings.sub.syncPluginLoadingCatalog": "Kontrola katalogu pluginů...",
  "settings.sub.syncPluginInstall": "Nainstalovat plugin",
  "settings.sub.syncPluginInstalling": "Instalace...",
  "settings.sub.syncPluginUnavailable":
    "Rozšířená synchronizace titulků není v katalogu pluginů dostupná.",
  "settings.sub.syncPluginBlocked":
    "Rozšířenou synchronizaci titulků nelze nainstalovat: {{reason}}",
  "settings.sub.syncPluginDisabled":
    "Rozšířená synchronizace titulků je nainstalovaná, ale vypnutá. Zapněte ji v Pluginech, chcete-li používat opravu časování.",
  "settings.sub.syncPluginInstalled": "{{plugin}} nainstalován",
  "settings.sub.syncPluginLoadFailed": "Katalog pluginů se nepodařilo načíst.",
  "settings.sub.syncPluginInstallFailed":
    "Nepodařilo se nainstalovat {{plugin}}: {{error}}",
  "settings.postProcessing": "Následné zpracování",
  "settings.pp.title": "Skripty následného zpracování",
  "settings.pp.description":
    "Skripty se spouštějí automaticky po importu médií. Blokující skripty běží v pořadí podle priority. Skripty typu fire-and-forget běží paralelně a nezpomalují proces importu.",
  "settings.pp.noScripts":
    "Nejsou nastaveny žádné skripty následného zpracování.",
  "settings.pp.name": "Název skriptu",
  "settings.pp.namePlaceholder": "např. Upozornit Plex",
  "settings.pp.descriptionLabel": "Popis",
  "settings.pp.descriptionPlaceholder": "Co tento skript dělá?",
  "settings.pp.scriptType": "Typ skriptu",
  "settings.pp.inline": "Vložený shell",
  "settings.pp.filePath": "Cesta k souboru",
  "settings.pp.inlineHelp":
    "Zadejte shellový příkaz, který se spustí přes shell serveru.",
  "settings.pp.filePathHelp":
    "Absolutní cesta ke spustitelnému skriptu na serveru. Argumenty, přesměrování a shellová expanze se nepoužívají.",
  "settings.pp.blocking": "Blokující",
  "settings.pp.fireAndForget": "Fire & Forget",
  "settings.pp.blockingHelp":
    "Běží v pořadí podle priority. Další skript čeká, až tento skončí.",
  "settings.pp.fireAndForgetHelp":
    "Běží na pozadí bez blokování importu. Výstup lze stále zachytit, pokud je to zapnuto.",
  "settings.pp.executionMode": "Režim spouštění",
  "settings.pp.timeout": "Časový limit (sekundy)",
  "settings.pp.timeoutLabel": "Časový limit",
  "settings.pp.priority": "Priorita",
  "settings.pp.priorityHelp": "Nižší číslo se spustí dřív.",
  "settings.pp.facets": "Platí pro",
  "settings.pp.debug": "Zachytávat výstup skriptu",
  "settings.pp.debugHelp":
    "Je-li zapnuto, uloží se stdout a stderr každého běhu, včetně běhů na pozadí. Užitečné pro řešení problémů.",
  "settings.pp.runHistory": "Historie spuštění",
  "settings.pp.noRuns": "Zatím žádná spuštění.",
  "settings.pp.outputNotCaptured":
    "Výstup nebyl zachycen. Pro zobrazení výstupu zapněte u tohoto skriptu ladicí režim.",
  "settings.pp.envHeading": "Proměnné prostředí",
  "settings.pp.envDescription":
    "Skripty dostávají JSON payload v SCRYER_METADATA, plus pomocné proměnné uvedené níže. Hodnoty mohou být null, pokud je Scryer nemá.",
  "settings.pp.created": "Skript vytvořen.",
  "settings.pp.updated": "Skript aktualizován.",
  "settings.pp.deleted": "Skript smazán.",
  "settings.pp.toggled": "Skript {{state}}.",
  "settings.pp.createNewScript": "Vytvořit nový skript",
  "settings.pp.confirmDiscardTitle":
    "Zahodit aktuální úpravy skriptu následného zpracování?",
  "settings.pp.confirmDiscardDescription":
    "Aktuální úpravy skriptu následného zpracování budou ztraceny.",
  "settings.pp.inlineShellConfirmTitle": "Povolit vložený shellový skript?",
  "settings.pp.inlineShellConfirmDescription":
    "Vložené skripty následného zpracování běží po importu přes shell serveru. Ukládejte jen skripty, které jste zkontrolovali a kterým důvěřujete.",
  "settings.pp.inlineShellConfirm": "Povolit vložený shell",
  "settings.plugins": "Pluginy",
  "settings.scripts": "Skripty",
  "settings.pluginRuntimeMemoryEstimate":
    "Odhadovaná spotřeba paměti pluginu za běhu",
  "settings.pluginsRefresh": "Obnovit katalog",
  "settings.pluginAutoUpdateEnabled":
    "Automaticky aktualizovat opravné verze pluginů",
  "settings.pluginAutoUpdateEnabledHelp":
    "Je-li zapnuto, plánované obnovení katalogu pluginů (nebo ruční spuštění této úlohy) aktualizuje oficiální pluginy na nejnovější opravnou verzi, např. 1.0.1 → 1.0.2, a nainstaluje optimalizované sestavení dané verze, pokud je dostupné. Vedlejší a hlavní verze zůstávají v rukou provozovatele.",
  "settings.pluginsUpdateAll": "Aktualizovat vše",
  "settings.pluginsUpdatingAll": "Aktualizace všeho…",
  "settings.host": "Host",
  "settings.port": "Port",
  "settings.baseUrl": "Základní URL",
  "settings.externalUrl": "Externí URL",
  "settings.externalUrlHint":
    "URL viditelná v prohlížeči, použitá pro odkazy na přehrávání v Jellyfin nebo Emby.",
  "label.watchIn": "Sledovat v",
  "settings.pluginsInstalled": "Nainstalováno",
  "settings.pluginsAvailable": "Dostupné",
  "settings.pluginsNoInstalled": "Nejsou nainstalovány žádné pluginy.",
  "settings.pluginsNoAvailable": "Žádný plugin neodpovídá aktuálnímu filtru.",
  "settings.pluginsNoPlugins":
    "Nebyly nalezeny žádné pluginy. Obnovte katalog a zkontrolujte dostupné pluginy.",
  "settings.pluginAllCategories": "Všechny kategorie",
  "settings.pluginCategoryIndexer": "Indexer",
  "settings.pluginCategoryUsenetIndexer": "Usenet indexer",
  "settings.pluginCategoryTorrentIndexer": "Torrent indexer",
  "settings.pluginCategoryDownloadClient": "Klient pro stahování",
  "settings.pluginCategoryArchiveExtractor": "Rozbalování archivů",
  "settings.pluginCategoryNotification": "Oznámení",
  "settings.pluginCategorySubtitleProvider": "Poskytovatel titulků",
  "settings.pluginOfficialOnly": "Jen oficiální",
  "settings.pluginInstall": "Instalovat",
  "settings.pluginInstalling": "Instalace…",
  "settings.pluginInstallDownloading": "Stahování",
  "settings.pluginInstallVerifying": "Ověřování",
  "settings.pluginInstallInstalling": "Instalace",
  "settings.pluginUninstall": "Odinstalovat",
  "settings.pluginRevertToBundled": "Vrátit k výchozí verzi",
  "settings.pluginRevert": "Vrátit",
  "settings.pluginUninstallWarning":
    "smazáním tohoto pluginu se smažou i všechna nastavení s ním spojená",
  "settings.pluginRevertToBundledWarning":
    "Tím se odstraní stažené přepsání pro „{{name}}“ a vrátí se vestavěný plugin dodávaný se Scryer.",
  "settings.pluginBuiltin": "Vestavěný",
  "settings.pluginInstalled": "Nainstalováno",
  "settings.pluginOfficial": "Oficiální",
  "settings.pluginVerifiedCommunity": "Ověřeno komunitou",
  "settings.pluginUnverified": "Neověřeno",
  "settings.pluginBeta": "Beta",
  "settings.pluginDeprecated": "Zastaralé",
  "settings.pluginBlocked": "Blokováno",
  "settings.pluginBlockedNotRunning": "Neběží",
  "settings.pluginSource": "Zdroj",
  "settings.pluginDocs": "Dokumentace",
  "settings.pluginInstallManually": "Instalovat ručně",
  "settings.pluginManualUploadTitle": "Nahrání lokálního souboru",
  "settings.pluginManualUploadHelp":
    "Nahrajte lokální sestavení pluginu pro rychlé testování. Scryer přijímá čisté soubory .wasm a balíčky .wasm.zst.",
  "settings.pluginManualUploadSelect": "Vybrat soubor pluginu",
  "settings.pluginManualUploadInstall": "Zkontrolovat a nainstalovat soubor",
  "settings.pluginManualUploadNoFile": "Nebyl vybrán žádný soubor pluginu.",
  "settings.pluginManualUploadFormats": "Povolené formáty: .wasm a .wasm.zst",
  "settings.pluginManualFileUnsupported":
    "Ruční nahrání pluginu musí být soubor .wasm nebo .wasm.zst.",
  "settings.pluginManualUploadConfirmTitle":
    "Nainstalovat lokální soubor pluginu?",
  "settings.pluginManualUploadConfirmDescription":
    "Chystáte se spustit neověřený lokální kód pluginu uvnitř Scryeru. Instalujte pouze soubory, které jste sami sestavili nebo kterým výslovně důvěřujete.",
  "settings.pluginManualUploadConfirmCheckbox":
    "Rozumím, že tento plugin spustí vlastní kód s oprávněními uvedenými v jeho manifestu.",
  "settings.pluginManualUploadConfirmAction": "Nainstalovat lokální plugin",
  "settings.pluginManualRepoTitle": "Instalace z GitHub repozitáře",
  "settings.pluginManualRepoHelp":
    "Nainstalujte neověřený plugin přímo z GitHub repozitáře pluginů, který publikuje formát katalogu pluginů Scryer.",
  "settings.pluginManualRepoUrl": "GitHub repozitář pluginu",
  "settings.pluginInspectManual": "Prozkoumat repozitář",
  "settings.pluginOverride": "Přepsat",
  "settings.pluginVersion": "v{{version}}",
  "settings.pluginUpdateAvailable": "Aktualizace: v{{version}}",
  "settings.pluginUpgrade": "Upgradovat na v{{version}}",
  "settings.pluginOptimizedBuildAvailable":
    "K dispozici je optimalizované sestavení",
  "settings.pluginInstallOptimizedBuild":
    "Nainstalovat optimalizované sestavení",
  "status.pluginInstalled": "Plugin „{{name}}“ nainstalován.",
  "status.pluginInstallAlreadyInProgress":
    "Plugin „{{name}}“ se už instaluje nebo upgraduje.",
  "status.pluginInstallFailedChecksumMismatch":
    "Instalace pluginu „{{name}}“ selhala: publikovaný artefakt už neodpovídá kontrolnímu součtu registru. Obnovte registr nebo to zkuste znovu později.",
  "status.pluginInstallFailedSdkMetadataMismatch":
    "Instalace pluginu „{{name}}“ selhala: publikovaný artefakt neodpovídá metadatům kompatibility SDK v registru. Obnovte registr nebo to zkuste znovu později.",
  "status.pluginInstallFailedWithReason":
    "Instalace pluginu „{{name}}“ selhala: {{reason}}",
  "status.pluginRevertedToBundled":
    "Plugin „{{name}}“ vrácen na zabudovanou verzi.",
  "status.pluginUninstalled": "Plugin „{{name}}“ odinstalován.",
  "status.pluginUpgraded": "Plugin „{{name}}“ upgradován na v{{version}}.",
  "status.pluginsUpgradeQueued":
    "Zařazeno do fronty upgradů pluginů: {{count}}.",
  "status.pluginAutoUpdateSettingsSaved":
    "Nastavení automatické aktualizace pluginů uloženo.",
  "status.pluginToggled": "Plugin „{{name}}“ {{state}}.",
  "status.catalogRefreshed": "Katalog pluginů obnoven.",
  "form.indexerNamePlaceholder": "Můj indexer Newznab",
  "form.indexerRateLimitSeconds": "Minimální počet sekund mezi dotazy",
  "form.indexerRateLimitSecondsPlaceholder": "Bez zpoždění",
  "form.indexerRateLimitSecondsInvalid":
    "Minimální počet sekund mezi dotazy musí být celé číslo, nebo prázdné pro žádné zpoždění.",
  "form.indexerMaxQueriesPerMinute": "Max. počet dotazů za minutu",
  "form.indexerMaxQueriesPerMinutePlaceholder": "Bez limitu",
  "form.indexerMaxQueriesPerMinuteInvalid":
    "Max. počet dotazů za minutu musí být celé číslo alespoň 1, nebo prázdné pro žádný limit.",
  "form.providerTypePlaceholder": "Typ poskytovatele",
  "form.subtitleProviderValidation": "Název a typ poskytovatele jsou povinné.",
  "form.subtitleProviderFacetsRequired":
    "Vyberte alespoň jeden typ obsahu. Scryer nikdy nevyhledává u poskytovatele, který žádný nemá.",
  "form.apiKeyInputPlaceholder": "klíč API",
  "form.apiKeyStoredPlaceholder": "uloženo (ponechte prázdné pro zachování)",
  "form.usernamePlaceholder": "uživatelské jméno",
  "form.passwordPlaceholder": "heslo",
  "form.newPasswordPlaceholder": "nové heslo",
  "form.indexerValidation":
    "Název, typ poskytovatele a základní URL jsou povinné.",
  "qualityProfile.noProfilesFound": "Nebyly nalezeny žádné profily.",
  "qualityProfile.createNewProfile": "Vytvořit nový profil kvality",
  "qualityProfile.editProfile": "Upravit profil kvality",
  "qualityProfile.confirmDiscardTitle":
    "Zahodit aktuální úpravy profilu kvality?",
  "qualityProfile.confirmDiscardDescription":
    "Vaše aktuální úpravy profilu kvality budou ztraceny.",
  "qualityProfile.profileNameLabel": "Název profilu",
  "qualityProfile.qualityTiersAndArchival":
    "Úrovně kvality a archivační kvalita",
  "qualityProfile.qualityPreference": "Preference kvality",
  "qualityProfile.qualityPreferenceHelp":
    "Nejvyšší preference je první. Preference úrovně má přednost před skóre. Nové úrovně se přidávají s nejnižší preferencí.",
  "qualityProfile.moveQualityTierUp": "Posunout {{value}} nahoru",
  "qualityProfile.moveQualityTierDown": "Posunout {{value}} dolů",
  "qualityProfile.dragQualityTier": "Přetažením změnit pořadí {{value}}",
  "qualityProfile.allowedQualityTiers": "Povolené úrovně kvality (vybrané)",
  "qualityProfile.noQualityTiersSelected":
    "Nejsou vybrány žádné úrovně kvality.",
  "qualityProfile.removeQualityTier": "Odebrat úroveň kvality {{value}}",
  "qualityProfile.addQualityTier": "Přidat {{value}} do výběru",
  "qualityProfile.availableQualityTiers": "Dostupné úrovně kvality",
  "qualityProfile.allQualityTiersSelected":
    "Vybrány všechny známé úrovně kvality.",
  "qualityProfile.archivalQuality": "Archivační kvalita",
  "qualityProfile.archivalQualityInfo":
    "Úroveň kvality, která se uchovává pro dlouhodobé uložení. Scryer může nejprve stáhnout vyšší kvalitu, pokud archivační kvalita zatím neexistuje, a později ji nahradit, jakmile je vydání s archivační kvalitou nalezeno.",
  "qualityProfile.qualityTiers": "Úrovně kvality",
  "qualityProfile.allowHdr": "HDR",
  "qualityProfile.detectedHdrAllowed": "HDR povoleno",
  "qualityProfile.detectedHdrAllowedInfo":
    "Povolit vydání rozpoznaná jako HDR (HDR10, HLG, Dolby Vision atd.). Pozn.: mnoho názvů vydání neuvádí HDR explicitně, takže to nezaručuje vyfiltrování HDR vydání.",
  "qualityProfile.allowUnknownQuality": "Povolena neznámá kvalita",
  "qualityProfile.allowUnknownQualityInfo":
    "Povolit vydání, u kterých parser nedokáže určit kvalitu. To může pomoci u neobvyklých enkódů, ale může povolit i shody s nižší jistotou.",
  "qualityProfile.dolbyVisionAllowed": "Dolby Vision",
  "qualityProfile.dolbyVisionInfo":
    "Povolit vydání Dolby Vision. Některé displeje nedokážou obsah Dolby Vision správně zobrazit, takže to nemusí vyhovovat všem.",
  "qualityProfile.preferRemux": "Preferovat Remux",
  "qualityProfile.preferRemuxInfo":
    "Remux jsou plnohodnotné kopie fyzických médií. To je užitečné hlavně s personou Audiophile.",
  "qualityProfile.allowBdDisk": "BD disk",
  "qualityProfile.allowBdDiskInfo":
    "Povolit vydání typu BD/obraz disku (ISO/Blu-ray disková vydání), aby prošla profilem.",
  "qualityProfile.allowDv": "Dolby Vision",
  "qualityProfile.ignoreProperRepack": "Ignorovat PROPER/REPACK",
  "qualityProfile.ignoreProperRepackInfo":
    "Nechat o výběru rozhodnout kvalitu a skóre bez automatické preference nebo ochrany revize. Pravidla hodnocení mohou vydání PROPER/REPACK stále odměňovat.",
  "qualityProfile.allowUpgrades": "Upgrady povoleny",
  "qualityProfile.allowUpgradesInfo":
    "Povolit pozdější nahrazení spravovaných kopií lepší shodou, nikdy ale nad vybranou archivační kvalitu.",
  "qualityProfile.sourceAllowlist": "Zdroje",
  "qualityProfile.videoCodecAllowlist": "Video kodeky",
  "qualityProfile.audioCodecAllowlist": "Audio kodeky",
  "qualityProfile.sourceAllowlistInfo":
    "Povolit pouze vydání odpovídající jedné z vybraných hodnot zdroje (například WEB-DL, HDTV).",
  "qualityProfile.sourceBlocklistDefault":
    "Ve výchozím nastavení nejsou blokovány žádné zdroje.",
  "qualityProfile.videoCodecAllowlistInfo":
    "Povolit pouze vydání enkódovaná některým z těchto video kodeků.",
  "qualityProfile.videoCodecBlocklistDefault":
    "Ve výchozím nastavení nejsou blokovány žádné video kodeky.",
  "qualityProfile.audioCodecAllowlistInfo":
    "Povolit pouze vydání s odpovídajícími audio kodeky.",
  "qualityProfile.audioCodecBlocklistDefault":
    "Ve výchozím nastavení nejsou blokovány žádné audio kodeky.",
  "qualityProfile.noSelectedItems": "Nic není vybráno.",
  "qualityProfile.aboutSection": "O profilu {{title}}",
  "qualityProfile.useDefaultQualityFallback": "Použít výchozí náhradní kvalitu",
  "qualityProfile.defaultCategoryProfiles": "Výchozí profily kategorií",
  "qualityProfile.scoringAndPreferences": "Hodnocení a preference",
  "qualityProfile.scoringAndPreferencesInfo":
    "Řídí, jak jsou vydání řazena a filtrována. Nastavte personu hodnocení, preference a chování při upgradu.",
  "qualityProfile.facetPersonaUseDefault": "Použít výchozí",
  "qualityProfile.scoringPersona": "Persona hodnocení",
  "qualityProfile.scoringPersonaInfo":
    "Přednastavené váhy hodnocení upřednostňující různé vlastnosti vydání. Pro většinu uživatelů se doporučuje Vyvážená.",
  "qualityProfile.personaBalanced": "Vyvážená",
  "qualityProfile.personaAudiophile": "Audiophile",
  "qualityProfile.personaEfficient": "Efektivní",
  "qualityProfile.personaCompatible": "Kompatibilní",
  "qualityProfile.scoringOverrides": "Přepsání hodnocení",
  "qualityProfile.scoringOverridesInfo":
    "Přepsat konkrétní chování hodnocení bez ohledu na vybranou personu. Ponechte na „Výchozí“ pro použití nastavení persony.",
  "qualityProfile.overrideAllowX265Non4k": "Povolit x265 mimo 4K",
  "qualityProfile.overrideAllowX265Non4kInfo":
    "Povolit vydání enkódovaná x265/HEVC v rozlišeních pod 4K. Některá zařízení mají s x265 v nižších rozlišeních potíže.",
  "qualityProfile.overrideBlockDvNoFallback": "Blokovat DV bez záložní vrstvy",
  "qualityProfile.overrideBlockDvNoFallbackInfo":
    "Blokovat vydání Dolby Vision bez záložní vrstvy HDR10, která se na displejích bez DV může zobrazovat zeleně nebo fialově.",
  "qualityProfile.overridePreferCompact": "Preferovat kompaktní enkódy",
  "qualityProfile.overridePreferCompactInfo":
    "Odměnit menší enkódy úsporné na šířku pásma. Užitečné při omezeném úložišti nebo streamování přes pomalejší připojení.",
  "qualityProfile.overridePreferLossless": "Preferovat bezztrátový zvuk",
  "qualityProfile.overridePreferLosslessInfo":
    "Přidat bonusové body vydáním s bezztrátovými zvukovými stopami (TrueHD, FLAC, DTS-HD MA).",
  "qualityProfile.overrideBlockUpscaled": "Blokovat AI upscalované",
  "qualityProfile.overrideBlockUpscaledInfo":
    "Blokovat vydání, která byla upscalovaná pomocí AI. Taková vydání mívají artefakty a nejsou ve skutečném nativním rozlišení.",
  "qualityProfile.cutoffTier": "Úroveň cutoffu",
  "qualityProfile.cutoffTierInfo":
    "Přestat upgradovat, jakmile je staženo vydání této (nebo lepší) úrovně kvality. Zabraňuje zbytečným opakovaným stahováním.",
  "qualityProfile.cutoffNone": "Žádný cutoff",
  "qualityProfile.minScoreToGrab": "Minimální skóre pro stažení",
  "qualityProfile.minScoreToGrabInfo":
    "Stahovat pouze vydání se skóre na této hranici nebo nad ní. Ponechte prázdné pro žádné minimum.",
  "qualityProfile.minScorePlaceholder": "Bez minima",
  "qualityProfile.confirmDeleteTitle": "Smazat profil kvality",
  "qualityProfile.confirmDeleteDescription":
    "Opravdu chcete smazat profil kvality „{{name}}“? Tuto akci nelze vrátit zpět.",
  "qualityProfile.deleteDisabledInUse":
    "Tento profil se používá. Před smazáním ho odeberte ze všech titulů a výchozích nastavení kategorií.",
  "episode.details": "Podrobnosti",
  "episode.search": "Hledat",
  "episode.blocklist": "Seznam blokovaných",
  "episode.overview": "Přehled",
  "episode.fileOnDisk": "Soubor na disku",
  "episode.airDate": "Datum vysílání",
  "episode.numberLabel": "Epizoda",
  "episode.quality": "Kvalita",
  "episode.missing": "Chybí",
  "episode.noBlockedReleases":
    "Pro tuto epizodu nejsou zaznamenána žádná blokovaná vydání.",
  "episode.untitledRelease": "Vydání bez názvu",
  "episode.filler": "Filler",
  "episode.recap": "Shrnutí",
  "episode.titleTba": "Bude upřesněno",
  "episode.special": "Speciál",
  "episode.ova": "OVA",
  "episode.ona": "ONA",
  "episode.alternate": "Alternativní",
  "seasonSection.seasonSearchResults": "Výsledky hledání pro {{season}}",
  "seasonSection.noEpisodeRecords":
    "Pro tuto řadu nejsou žádné záznamy epizod.",
  "seriesOverview.selectColumn": "Vybrat",
  "seriesOverview.selectEpisodeForDelete": "Vybrat {{name}}",
  "seriesOverview.selectSeasonForDelete": "Vybrat všechny epizody v {{name}}",
  "seriesOverview.clearEpisodeSelection": "Zrušit výběr",
  "seriesOverview.deleteSelectedEpisodeFiles": "Smazat vybrané ({{count}})",
  "seriesOverview.deleteEpisodeFilesTitle": "Smazat soubory vybraných epizod",
  "seriesOverview.deleteEpisodeFilesDescription":
    "Tímto se trvale smažou soubory na disku pro vybrané epizody. Záznamy epizod zůstanou zachovány.",
  "seriesOverview.deleteEpisodeFilesSummary":
    "Souborů: {{files}} · epizod: {{episodes}}",
  "seriesOverview.deleteEpisodeFilesPreviewFailures":
    "Souborů, které se nepodařilo zobrazit v náhledu: {{count}}. Jejich smazání může selhat.",
  "mediaFile.pendingScan": "Čeká na kontrolu",
  "mediaFile.scanFailed": "Kontrola selhala",
  "mediaFile.reviewRequired": "Vyžaduje kontrolu",
  "mediaFile.discReviewInventory":
    "Tituly z poslední inspekce. Uložený výběr a mapování epizod zůstávají v platnosti, dokud neuložíte ověřenou volbu.",
  "mediaFile.score": "Skóre {{score}}",
  "scoring.current": "Aktuální skóre",
  "scoring.score": "Skóre",
  "scoring.previous": "Předchozí skóre",
  "scoring.change": "Změna",
  "scoring.decision": "Rozhodnutí",
  "scoring.manualOverride": "Ruční výběr",
  "scoring.needsRefresh": "Vyžaduje obnovení",
  "scoring.timeSensitive": "Časově citlivé",
  "scoring.breakdown": "Rozpis skóre",
  "scoring.customRule": "Vlastní pravidlo",
  "scoring.systemRule": "Systémové pravidlo",
  "scoring.rejected": "Zamítnuto",
  "scoring.refreshOne": "Obnovit skóre",
  "scoring.refreshSelected": "Obnovit skóre",
  "scoring.refreshFailed":
    "Skóre se nepodařilo obnovit. Zkontrolujte výběr a oprávnění.",
  "mediaFile.primary": "Primární",
  "mediaFile.additional": "Doplňkové",
  "mediaFile.makePrimary": "Nastavit jako primární",
  "mediaFile.audioCount": "Zvuk: {{count}}",
  "mediaFile.commentary": "Komentář",
  "mediaFile.audioDescription": "Audiopopis",
  "mediaFile.hearingImpaired": "Pro neslyšící",
  "mediaFile.program": "Program {{id}}",
  "mediaFile.discTitle": "Titul pro přehrávání disku",
  "mediaFile.discReviewAction": "Disk vyžaduje pozornost",
  "mediaFile.discOutcomeAutomatic":
    "Přehrává se titul {{id}} ({{duration}}), vybraný automaticky jako nejdelší úplný titul.",
  "mediaFile.discOutcomeManual":
    "Přehrává se titul {{id}} ({{duration}}), vybraný vámi. Automatický výběr je pro tento disk vypnutý.",
  "mediaFile.discOutcomeUnselected":
    "Nepodařilo se vybrat žádný přehratelný titul. Vyberte jeden níže.",
  "mediaFile.discOutcomeIncomplete":
    "Inspekce vybraného titulu se nedokončila ({{status}}). Vyberte úplný titul nebo soubor znovu zkontrolujte.",
  "mediaFile.discChooseTitle": "Který titul se má přehrávat",
  "mediaFile.discAutomaticCurrent": "Aktuálně titul {{id}}",
  "mediaFile.discTitleChapterCount": "Kapitol: {{count}}",
  "mediaFile.discTitleAudioCount": "Zvukových stop: {{count}}",
  "mediaFile.discTitleNotSelectable": "Inspekce {{status}}",
  "mediaFile.discTitleMissing": "Na disku už není",
  "mediaFile.discPlaying": "Přehrává se",
  "mediaFile.discUseTitle": "Použít tento titul",
  "mediaFile.discStreamsAndChapters": "Stopy a kapitoly",
  "mediaFile.discTechnicalDetails": "Technické podrobnosti",
  "mediaFile.discTitleIdentity": "Titul {{id}}",
  "mediaFile.discInspectTitles": "Prozkoumat stopy a kapitoly titulu",
  "mediaFile.discAngleInventory":
    "Úhlů pohledu: {{count}}. Inspekce médií pokrývá první dostupný úhel pohledu.",
  "mediaFile.discTitleAliases": "Rovnocenná ID titulu: {{ids}}",
  "mediaFile.discTitleBudgetExhausted":
    "Inspekce tohoto titulu vyčerpala svůj rozpočet na sondování.",
  "mediaFile.discTitleStreams": "Stopy titulu",
  "mediaFile.discTitleStreamsUnknown":
    "Pro tento titul se nepodařilo získat žádná metadata stop.",
  "mediaFile.discTitleChapters": "Kapitoly titulu",
  "mediaFile.discTitleChaptersUnknown":
    "Pro tento titul se nepodařilo získat žádná metadata kapitol.",
  "mediaFile.discChapterIdentity": "Kapitola {{id}}",
  "mediaFile.discEpisodeMapping": "Mapovat tituly disku na epizody",
  "mediaFile.discMappingScope":
    "Zvolte jednu epizodu pro každý autorský titul přehrávání. Nenamapované tituly nezpřístupní žádné epizody. ISO zůstává jedním souborem.",
  "mediaFile.discMappingSave": "Uložit mapování epizod",
  "mediaFile.discMappingFailed": "Mapování epizod se nepodařilo uložit",
  "mediaFile.discMappingLoading": "Načítání epizod…",
  "mediaFile.discMappingNone": "Žádná epizoda",
  "mediaFile.discAutomatic": "Automaticky nejdelší platný titul",
  "mediaFile.discSaving": "Probíhá inspekce titulu…",
  "mediaFile.discSelectionFailed": "Výběr titulu disku se nepodařilo uložit",
  "mediaFile.analysisHeading": "Inspekce médií {{status}}",
  "mediaFile.analysisLatestInspection":
    "Poslední inspekce {{status}} v {{time}}",
  "mediaFile.analysisPreviousRetained":
    "Předchozí uložená inspekce zůstává zachována.",
  "mediaFile.analysisNoneSaved":
    "Pro tento soubor nebyla uložena žádná inspekce.",
  "mediaFile.analysisBudgetExhausted": "Rozpočet na inspekci byl vyčerpán.",
  "mediaFile.analysisSummary":
    "Trvání {{duration}} · celkově {{overall}} · video {{video}}",
  "mediaFile.analysisBudgetStopped":
    "Inspekce se zastavila u svého rozpočtu na sondování.",
  "mediaFile.analysisProgram": "Program {{id}}",
  "mediaFile.analysisProgramStreams": "stopy {{ids}}",
  "mediaFile.analysisSelected": "vybráno",
  "mediaFile.analysisChapters": "Kapitoly",
  "mediaFile.analysisAttachments": "Přílohy",
  "mediaFile.analysisUnknownType": "neznámý typ",
  "mediaFile.analysisBytes": "{{count}} B",
  "mediaFile.analysisCaptionService":
    "{{standard}} titulkovací služba {{number}} · {{language}}",
  "mediaFile.analysisUnknownLanguage": "neznámý jazyk",
  "mediaFile.streamUnknownCodec": "neznámý kodek",
  "mediaFile.streamLanguage": "Jazyk",
  "mediaFile.streamProfileLevel": "Profil / úroveň",
  "mediaFile.streamBitrate": "Datový tok",
  "mediaFile.streamEstimatedBitrate": "Odhadovaný datový tok",
  "mediaFile.streamDimensions": "Rozměry",
  "mediaFile.streamPixelFormat": "Formát pixelů / hloubka",
  "mediaFile.streamFieldOrder": "Pořadí půlsnímků",
  "mediaFile.streamAspectRotation": "Poměr stran / rotace",
  "mediaFile.streamDeclaredFrameRate": "Deklarovaná snímková frekvence",
  "mediaFile.streamObservedFrameRate": "Pozorovaná snímková frekvence",
  "mediaFile.streamVfrUnknown": "proměnná snímková frekvence neznámá",
  "mediaFile.streamVfrObserved": "pozorována proměnná snímková frekvence",
  "mediaFile.streamVfrNotObserved": "konstantní snímková frekvence",
  "mediaFile.streamHdr": "Signály HDR",
  "mediaFile.streamNoHdr": "žádné",
  "mediaFile.streamColor": "Primární barvy / přenos / matice",
  "mediaFile.streamContentLight": "MaxCLL / MaxFALL",
  "mediaFile.streamMasteringLuminance": "Masterovací jas",
  "mediaFile.streamDolbyVision": "Profil Dolby Vision / kompatibilita",
  "mediaFile.streamChannels": "Kanály / rozložení",
  "mediaFile.streamSampleRate": "Vzorkovací frekvence / formát",
  "mediaFile.streamSampleDepth": "Bitová hloubka vzorku",
  "mediaFile.streamUnknownLayout": "neznámé rozložení",
  "mediaFile.diagnose": "Prozkoumat strukturální hlavičky",
  "mediaFile.diagnosing": "Probíhá kontrola hlaviček…",
  "mediaFile.diagnosticsFailed": "Tento soubor se nepodařilo zkontrolovat",
  "mediaFile.diagnosticsScope":
    "Omezené vzorkování hlaviček neověřuje každý paket ani dekódovaný obraz a zvukový obsah.",
  "mediaFile.diagnosticsRead":
    "Přečteno bajtů: {{bytes}} za {{milliseconds}} ms",
  "mediaFile.diagnosticsSamples":
    "Vzorkováno přenosových paketů: {{packets}}, hlaviček snímků: {{frames}}",
  "mediaFile.diagnosticsCoverage": "Prozkoumáno {{percent}} % zdrojových bajtů",
  "mediaFile.diagnosticsRanges": "Prozkoumané rozsahy bajtů",
  "mediaFile.diagnosticsTruncated":
    "Některé rozsahy nebo upozornění byly vynechány, protože byl dosažen limit hlášení.",
  "mediaFile.subtitleCount": "Titulky: {{count}}",
  "mediaFile.delete": "Smazat soubor",
  "subtitle.search": "Hledat titulky",
  "subtitle.searching": "Hledání…",
  "subtitle.download": "Stáhnout",
  "subtitle.downloading": "Stahování…",
  "subtitle.noResults": "Nenalezeny žádné titulky",
  "subtitle.score": "Skóre",
  "subtitle.scoreWithValue": "Skóre: {{score}}",
  "subtitle.hearingImpaired": "HI",
  "subtitle.forced": "Vynucené",
  "subtitle.aiTranslated": "AI",
  "subtitle.machineTranslated": "Strojový překlad",
  "subtitle.external": "Externí",
  "subtitle.manualSearch": "Ruční hledání",
  "subtitle.providersRequiredTitle": "Vyžadován poskytovatel titulků",
  "subtitle.providersRequiredBody":
    "Před hledáním titulků přidejte a zapněte alespoň jednoho poskytovatele titulků.",
  "subtitle.providersRequiredAction": "Otevřít poskytovatele titulků",
  "subtitle.providerUnavailableTitle": "Služba titulků nedostupná",
  "subtitle.providerUnavailableBody":
    "Služba titulků je pro tento soubor momentálně nedostupná.",
  "subtitle.subtitlesDisabledTitle": "Stahování titulků je vypnuté",
  "subtitle.subtitlesDisabledBody":
    "Před hledáním titulků zapněte stahování titulků v nastavení titulků.",
  "subtitle.blocklist": "Seznam blokovaných",
  "subtitle.blocklisted": "Titulky přidány na seznam blokovaných",
  "subtitle.deleted": "Titulky smazány",
  "subtitle.onDisk": "Na disku",
  "subtitle.releaseInfo": "Vydání",
  "subtitle.provider": "Poskytovatel",
  "subtitle.flags": "Příznaky",
  "subtitle.synced": "Synchronizováno",
  "settings.fillerPolicyLabel": "Zásady pro filler epizody",
  "settings.fillerPolicyDownloadAll": "Stahovat všechny epizody",
  "settings.fillerPolicySkipFiller": "Přeskočit filler epizody",
  "settings.recapPolicyLabel": "Zásady pro epizody se shrnutím",
  "settings.recapPolicyDownloadAll": "Stahovat všechny epizody",
  "settings.recapPolicySkipRecap": "Přeskočit epizody se shrnutím",
  "settings.releaseNumberingLabel": "Číslování vydání",
  "settings.releaseNumberingAuto": "Automaticky",
  "settings.releaseNumberingOfficial": "Oficiální pořadí",
  "settings.releaseNumberingAlternate": "Alternativní pořadí",
  "settings.releaseNumberingDvd": "Pořadí dle DVD",
  "settings.monitorSpecialsLabel": "Sledovat speciály a OVA",
  "settings.monitorSpecialsDescription":
    "Při přidání anime automaticky sledovat speciální epizody a OVA",
  "settings.interSeasonMoviesLabel": "Filmy mezi řadami",
  "settings.interSeasonMoviesDescription":
    "Vytvářet narativní položky pro kanonické filmy mezi řadami",
  "settings.monitorFillerMoviesLabel": "Sledovat filler filmy",
  "settings.monitorFillerMoviesDescription":
    "Zahrnout do fronty stahování filmy klasifikované jako filler",
  "settings.importModeLabel": "Režim importu",
  "settings.importModeDescription":
    "Zvolte, zda Scryer po ověřeném importu do knihovny zachová soubor na straně stahování, nebo ho odstraní.",
  "settings.importModeHardlinkCopy": "Pevný odkaz / Kopie",
  "settings.importModeMove": "Přesunout",
  "settings.setPermissionsLinuxLabel": "Nastavit oprávnění Unix",
  "settings.setPermissionsLinuxDescription":
    "Použít nastavený režim souboru a skupinu u importů na Unixu. Číselné skupiny se považují za GID.",
  "settings.fileChmodLabel": "Chmod souboru",
  "settings.folderChmodLabel": "Chmod složky",
  "settings.chownGroupLabel": "Vlastnící skupina",
  "settings.fileChmodDeriveFromFolder": "Odvodit z chmodu složky",
  "settings.chmodPresetOwnerWriteEveryoneRead": "Vlastník zápis, ostatní čtení",
  "settings.chmodPresetOwnerGroupWriteOtherRead":
    "Vlastník a skupina zápis, ostatní čtení",
  "settings.chmodPresetOwnerGroupWrite": "Vlastník a skupina zápis",
  "settings.chmodPresetOwnerWriteGroupRead": "Vlastník zápis, skupina čtení",
  "settings.chmodPresetEveryoneWrite": "Všichni zápis",
  "settings.chmodPresetCustom": "Vlastní",
  "settings.sidecarFilesTitle": "Doprovodné soubory",
  "settings.identityStorageTitle": "Identita a úložiště",
  "settings.mediaProfilesTitle": "Mediální profily",
  "settings.mediaProfilesHelp":
    "Ponechte na „Zdědit výchozí z katalogu“ pro sledování katalogu, nebo přepište pro jednotlivé knihovny. Výsledná hodnota se zobrazuje pod každým ovládacím prvkem.",
  "settings.importBehaviorTitle": "Chování při importu",
  "settings.libraryUnsavedChanges": "Neuložené změny",
  "settings.nfoWriteOnImportLabel": "Zapsat NFO při importu",
  "settings.nfoWriteOnImportDescription":
    "Zapisovat metadatové soubory .nfo kompatibilní s Kodi/Jellyfin/Emby vedle importovaných médií; existující soubor .nfo se nikdy nepřepíše",
  "settings.plexmatchWriteOnImportLabel": "Zapsat .plexmatch při importu",
  "settings.plexmatchWriteOnImportDescription":
    "Zapisovat nápovědné soubory Plex pro přesné párování seriálů bez odhadování podle názvu",
  "settings.minAvailabilityLabel": "Minimální dostupnost",
  "settings.minAvailability.default": "Použít výchozí z knihovny",
  "settings.minAvailability.announced": "Oznámeno",
  "settings.minAvailability.in_cinemas": "V kinech",
  "settings.minAvailability.released": "Vydáno",
  "status.titleRequired": "Název je povinný.",
  "status.facetRequired": "Oblast je povinná.",
  "status.catalogAddSuccess": "{{name}} přidáno do katalogu.",
  "status.catalogAddSuccessAutoSearch":
    "{{name}} přidáno do katalogu. Automatické vyhledávání zařazeno do fronty.",
  "status.requestSubmitted": "Žádost o {{name}} byla odeslána.",
  "status.queueSuccess": "{{name}} zařazeno do fronty",
  "status.primaryMovieFileUpdated": "Primární soubor filmu aktualizován",
  "status.blocklistEntryCleared": "Blokované vydání odebráno",
  "status.episodeFilesDeleteQueued":
    "Zařazeno do fronty smazání souborů epizod: {{count}}",
  "status.episodeFilesDeleted": "Smazáno souborů epizod: {{count}}",
  "status.mediaFileDeleteQueued":
    "Smazání mediálního souboru zařazeno do fronty",
  "status.mediaFileDeleted": "Mediální soubor smazán",
  "status.queuedLatest": "Nejnovější vydání pro {{name}} zařazeno do fronty",
  "status.nothingFound": "Nebyly nalezeny žádné výsledky.",
  "status.searchingTvdb": "Hledání metadat TVDB pro „{{query}}“.",
  "status.foundTvdb": "Nalezeno výsledků TVDB: {{count}}.",
  "status.foundNzb": "Nalezeno výsledků NZBGeek: {{count}}.",
  "status.renamePreviewGenerated":
    "Náhled přejmenování připraven. Soubory k přejmenování: {{renamable}} (celkem: {{total}}).",
  "status.renameQueued":
    "Přejmenování probíhá na pozadí; průběh se zobrazuje v Aktivitě.",
  "status.releaseMissingCandidateToken":
    "Vybranému vydání chybí ověřený token kandidáta.",
  "status.searchResultsExpired":
    "Platnost výsledků hledání vypršela, vyhledejte prosím znovu",
  "status.deleteCatalogConfirm":
    "Odebrat {{name}} z katalogu? Pomocí volby níže lze také odstranit lokální soubory.",
  "status.titleDeleted": "{{name}} odebráno z katalogu.",
  "status.bulkMonitorSuccess": "Sledováno titulů: {{count}}.",
  "status.bulkMonitorPartial":
    "Sledováno titulů: {{count}}; selhalo: {{failed}}.",
  "status.bulkMonitorFailed":
    "Nepodařilo se nastavit sledování vybraných titulů.",
  "status.bulkUnmonitorSuccess": "Zrušeno sledování titulů: {{count}}.",
  "status.bulkUnmonitorPartial":
    "Zrušeno sledování titulů: {{count}}; selhalo: {{failed}}.",
  "status.bulkUnmonitorFailed":
    "Nepodařilo se zrušit sledování vybraných titulů.",
  "status.bulkTitleUpdateSuccess": "Aktualizováno titulů: {{count}}.",
  "status.bulkTitleUpdatePartial":
    "Aktualizováno titulů: {{count}}; selhalo: {{failed}}.",
  "status.bulkTitleUpdateFailed": "Nepodařilo se aktualizovat vybrané tituly.",
  "status.bulkDeletePreviewFailed":
    "Nepodařilo se připravit náhled smazání pro vybrané tituly: {{failed}}.",
  "status.bulkTitleDeleteFailed": "Nepodařilo se smazat vybrané tituly.",
  "status.bulkRenamePreviewFailed":
    "Nepodařilo se zobrazit náhled přejmenování pro tituly: {{failed}}.",
  "status.bulkRenameFailed": "Nepodařilo se přejmenovat vybrané tituly.",
  "status.bulkRenameQueued":
    "Přejmenování titulů probíhá na pozadí: {{count}}; průběh se zobrazuje v Aktivitě.",
  "status.bulkRenameForbidden":
    "Nemáte oprávnění přejmenovat tituly ve všech vybraných knihovnách.",
  "status.titleMatchUpdated": "Shoda pro {{name}} aktualizována.",
  "status.languageChanged": "Jazyk rozhraní nastaven na {{language}}.",
  "status.languageSaveFailed":
    "Jazyk se nepodařilo uložit do vašeho profilu; ostatní prohlížeče si ponechají svůj aktuální jazyk.",
  "status.catalogViewSaveFailed":
    "Toto rozvržení se nepodařilo uložit do vašeho profilu; platí pouze v tomto prohlížeči.",
  "status.editingIndexer": "Úprava indexeru {{name}}",
  "status.editingDownloadClient": "Úprava klienta pro stahování {{name}}",
  "status.editingSubtitleProvider": "Úprava poskytovatele titulků {{name}}",
  "status.deletingUser": "Smazat uživatele {{name}}?",
  "status.deletingUserListsWarning":
    "Smažou se i seznamy, které tento uživatel sleduje, a jejich historie synchronizace.",
  "status.userDeleted": "Uživatel {{name}} smazán.",
  "status.userLoginEnabled": "Přihlašování pro {{name}} zapnuto.",
  "status.userLoginDisabled": "Přihlašování pro {{name}} vypnuto.",
  "status.deletingIndexer": "Smazat indexer {{name}}?",
  "status.deletingSubtitleProvider": "Smazat poskytovatele titulků {{name}}?",
  "status.userRequired": "Uživatelské jméno a heslo jsou povinné.",
  "status.passwordRequired": "Heslo je povinné.",
  "status.failedToCreate": "Vytvoření položky selhalo.",
  "status.failedToDelete": "Smazání položky selhalo.",
  "status.failedToLoad": "Načtení dat selhalo.",
  "status.failedToUpdate": "Aktualizace položky selhala.",
  "status.managePermissionsRequired":
    "Pro úpravu oprávnění je vyžadováno oprávnění Spravovat oprávnění.",
  "status.indexerCreated": "Indexer uložen.",
  "status.indexerUpdated": "Indexer aktualizován.",
  "status.indexerSynced":
    "Synchronizovány indexery spravované uživatelem {{name}}.",
  "status.mismatchRecoveryQueued":
    "Zařazeno do fronty položek obnovy neshody: {{count}}.",
  "status.titleScanSuccess":
    "Kontrola dokončena. Importováno: {{imported}}, přeskočeno: {{skipped}}, nespárováno: {{unmatched}}.",
  "status.titleMonitoringEnabled": "Sledování titulu zapnuto.",
  "status.titleMonitoringDisabled": "Sledování titulu vypnuto.",
  "status.indexerDeleted": "Indexer {{name}} smazán.",
  "plugins.failure.invalidConfig":
    "Plugin nemohl použít toto nastavení. Zkontrolujte ho a zkuste to znovu.",
  "plugins.failure.authFailed":
    "Služba odmítla přihlašovací údaje. Zkontrolujte klíč API, uživatelské jméno nebo heslo.",
  "plugins.failure.rateLimited":
    "Služba požádala Scryer, aby zpomalil. Zkuste to znovu za chvíli.",
  "plugins.failure.upstreamUnavailable":
    "Službu se nepodařilo kontaktovat nebo neodpověděla. Zkontrolujte adresu a zkuste to znovu.",
  "plugins.failure.unsupported": "Plugin toto nepodporuje.",
  "plugins.failure.temporary":
    "Plugin narazil na dočasný problém. Zkuste to znovu za chvíli.",
  "plugins.failure.permanent":
    "Plugin nemohl tento požadavek dokončit. Opakování nepomůže, dokud se něco nezmění.",
  "plugins.failure.hostFailure":
    "Plugin neočekávaně skončil. Podrobnosti najdete v protokolu Scryeru.",
  "plugins.failure.timedOut": "Plugin odpovídal příliš dlouho.",
  "plugins.failure.resourceLimit": "Pluginu došla paměť.",
  "plugins.failure.withDetail": "{{reason}} Podrobnosti: {{detail}}",
  "plugins.failure.invalidUsernamePassword":
    "Služba odmítla uživatelské jméno nebo heslo. Zkontrolujte obojí a zkuste to znovu.",
  "plugins.failure.invalidApiKey":
    "Služba odmítla klíč API. Zkontrolujte klíč a zkuste to znovu.",
  "plugins.failure.invalidPasskey":
    "Služba odmítla přístupový klíč. Zkontrolujte ho a zkuste to znovu.",
  "plugins.failure.invalidCookie":
    "Služba odmítla soubor cookie relace. Zkopírujte z prohlížeče nový cookie a zkuste to znovu.",
  "plugins.failure.captchaRequired":
    "Služba vyžaduje CAPTCHU. Přihlaste se jednou na jejím webu a zkuste to znovu.",
  "plugins.failure.twoFactorRequired":
    "Služba pro toto přihlášení vyžaduje dvoufaktorový kód. Použijte místo toho klíč API nebo aplikační heslo.",
  "plugins.failure.accountBanned":
    "Služba tento účet zablokovala. Pro vyřešení kontaktujte přímo službu.",
  "plugins.failure.accountDisabled":
    "Služba tento účet vypnula. Zkontrolujte účet na webu služby.",
  "plugins.failure.endpointNotFound":
    "Službu se na této adrese nepodařilo najít. Zkontrolujte URL a základní cestu.",
  "plugins.failure.tlsError":
    "Zabezpečené připojení ke službě selhalo. Zkontrolujte adresu a její certifikát.",
  "plugins.failure.invalidResponse":
    "Služba poslala odpověď, kterou plugin nedokázal přečíst. Zkontrolujte, zda adresa směřuje na správnou službu.",
  "plugins.failure.quotaExceeded":
    "Účet vyčerpal svou kvótu u této služby. Počkejte na její obnovení nebo zvyšte limit.",
  "plugins.failure.itemNotFound": "Služba již tuto položku nemá.",
  "plugins.failure.downloadRejected":
    "Klient pro stahování toto stažení odmítl.",
  "plugins.failure.pathNotWritable":
    "Plugin nemohl zapisovat do nastavené složky. Zkontrolujte, zda existuje a je zapisovatelná.",
  "plugins.failure.unknown": "Plugin nahlásil problém: {{code}}.",
  "status.testingIndexerConnection": "Testování připojení…",
  "status.indexerConnectionTestPassed": "Test připojení indexeru úspěšný.",
  "status.indexerConnectionTestFailed": "Test připojení indexeru selhal.",
  "status.testingSubtitleProviderConnection":
    "Testuje se připojení k poskytovateli titulků…",
  "status.subtitleProviderConnectionTestPassed":
    "Test připojení k poskytovateli titulků byl úspěšný.",
  "status.subtitleProviderConnectionTestFailed":
    "Test připojení k poskytovateli titulků selhal.",
  "status.ruleCreated": "Pravidlo uloženo.",
  "status.ruleUpdated": "Pravidlo aktualizováno.",
  "status.ruleDeleted": "Pravidlo {{name}} smazáno.",
  "status.ruleToggled": "Pravidlo {{name}} {{state}}.",
  "status.editingRule": "Úprava pravidla {{name}}",
  "status.deletingRule": "Smazat pravidlo {{name}}?",
  "status.mediaServerCreated": "Připojení mediálního serveru uloženo.",
  "status.mediaServerUpdated": "Připojení mediálního serveru aktualizováno.",
  "status.mediaServerDeleted": "Mediální server {{name}} smazán.",
  "status.plexServersDiscovered": "Servery Plex načteny.",
  "status.editingMediaServer": "Úprava mediálního serveru {{name}}",
  "status.deletingMediaServer": "Smazat mediální server {{name}}?",
  "status.testingMediaServerConnection":
    "Testuje se připojení k serveru {{server}}…",
  "status.mediaServerConnectionTestPassed":
    "Test připojení k serveru {{server}} byl úspěšný.",
  "status.mediaServerConnectionTestFailed":
    "Test připojení k serveru {{server}} selhal.",
  "status.downloadClientCreated": "Klient pro stahování uložen.",
  "status.downloadClientUpdated": "Klient pro stahování aktualizován.",
  "status.downloadClientDeletedWithMappings":
    "Klient pro stahování {{name}} smazán. Indexerů vráceno na automatický režim: {{count}}.",
  "status.indexerDownloadClientMappingSaving":
    "Ukládá se směrování klienta pro stahování…",
  "status.indexerDownloadClientMappingSaved":
    "Směrování klienta pro stahování uloženo pro {{name}}.",
  "status.testingDownloadClient": "Testuje se připojení k {{client}}…",
  "status.downloadClientConnectionTestPassed":
    "Test připojení k {{client}} byl úspěšný.",
  "status.downloadClientConnectionTestFailed":
    "Test připojení k {{client}} selhal.",
  "status.autoSearchNoCandidates":
    "Pro {{name}} nebylo nalezeno žádné vydání: žádný indexer nevrátil kandidáta. Zkontrolujte kategorie směrování indexerů a stav indexerů.",
  "status.autoSearchAllRejected":
    "Pro {{name}} nebylo nalezeno žádné vydání: automatické vyhledávání odmítlo všechny kandidáty ({{count}}). {{reasons}}",
  "status.queueFailed": "Operace s frontou selhala.",
  "status.apiError": "Požadavek selhal",
  "status.permissionDenied": "K této akci nemáte oprávnění.",
  "user.created": "Uživatel {{name}} vytvořen.",
  "user.permissionsUpdated": "Oprávnění pro {{name}} aktualizována.",
  "user.passwordUpdated": "Heslo pro {{name}} aktualizováno.",
  "user.mfaReset": "Ověřovací faktory pro {{name}} resetovány.",
  "profile.accountInfo": "Účet",
  "profile.appearance": "Vzhled",
  "profile.appearanceDescription":
    "Přizpůsobte si, jak Scryer vypadá pro váš účet.",
  "profile.highlightColor": "Zvýrazňovací barva",
  "profile.highlightColorHelp":
    "Používá se u výběru, aktivní navigace, odznaků a hlavních akcí.",
  "profile.highlightColorCustom": "Vlastní barva",
  "profile.highlightColorModel": "Barevný model",
  "profile.highlightColorApply": "Použít barvu",
  "profile.highlightColorCancel": "Zrušit",
  "profile.highlightColorSaved": "Zvýrazňovací barva aktualizována.",
  "profile.highlightColorSaveFailed":
    "Aktualizace zvýrazňovací barvy se nezdařila.",
  "color.indigo": "Indigová",
  "color.violet": "Fialová",
  "color.sky": "Nebeská",
  "color.teal": "Petrolejová",
  "color.emerald": "Smaragdová",
  "color.green": "Zelená",
  "color.amber": "Jantarová",
  "color.cyan": "Azurová",
  "color.rose": "Růžová",
  "color.red": "Červená",
  "color.fuchsia": "Fuchsiová",
  "color.magenta": "Purpurová",
  "color.crab": "Krabí",
  "profile.glow": "Záře",
  "profile.glowDescription":
    "Zobrazit jemnou záři v barvě zvýraznění za rozhraním.",
  "profile.glowToggle": "Zobrazit záři",
  "profile.glowSaved": "Nastavení záře uloženo.",
  "profile.glowSaveFailed": "Aktualizace nastavení záře se nezdařila.",
  "profile.changePassword": "Změnit heslo",
  "profile.currentPassword": "Současné heslo",
  "profile.newPassword": "Nové heslo",
  "profile.confirmPassword": "Potvrzení hesla",
  "profile.passwordMismatch": "Hesla se neshodují",
  "profile.passwordUpdated": "Heslo bylo úspěšně aktualizováno.",
  "profile.passkeys": "Přístupové klíče",
  "profile.passkeysDescription":
    "Přidejte nebo odeberte přístupové klíče pro rychlejší přihlášení na vašich zařízeních.",
  "profile.passkeysEmpty": "Zatím nebyl přidán žádný přístupový klíč.",
  "profile.passkeyAdd": "Přidat přístupový klíč",
  "profile.passkeyAdding": "Přidává se přístupový klíč…",
  "profile.passkeyLabel": "Přístupový klíč",
  "profile.passkeyCreatedAt": "Vytvořeno",
  "profile.passkeyLastUsedAt": "Naposledy použito",
  "profile.passkeyNeverUsed": "Nikdy nepoužito",
  "profile.passkeyAdded": "Přístupový klíč přidán.",
  "profile.passkeyDeleted": "Přístupový klíč smazán.",
  "profile.passkeyDeleteConfirmTitle": "Smazat přístupový klíč?",
  "profile.passkeyDeleteConfirmDescription":
    "Tento přístupový klíč již nebude možné použít k přihlášení k vašemu účtu.",
  "profile.passkeyDeleteFailed": "Smazání přístupového klíče se nezdařilo.",
  "profile.passkeyOperationFailed": "Požadavek na přístupový klíč selhal.",
  "profile.totp": "Aplikace autentikátoru",
  "profile.totpDescription":
    "Použijte jednorázový kód z aplikace 1Password nebo jiného autentikátoru pro dvoufázové ověření.",
  "profile.totpStartEnrollment": "Nastavit autentikátor",
  "profile.totpOpenSetupLink": "Otevřít odkaz pro nastavení",
  "profile.totpSecret": "Klíč pro nastavení",
  "profile.totpCode": "Jednorázový kód",
  "profile.totpVerifyAndEnable": "Ověřit a zapnout",
  "profile.totpEnabled": "Aplikace autentikátoru zapnuta.",
  "profile.totpDisabled": "Aplikace autentikátoru odpojena.",
  "profile.totpEnabledAt": "Zapnuto",
  "profile.totpLastUsedAt": "Naposledy použito",
  "profile.totpRecoveryRemaining": "Zbývající záložní kódy",
  "profile.totpRecoveryCodes": "Záložní kódy",
  "profile.totpActionDescription":
    "Pro správu tohoto autentikátoru zadejte kód z autentikátoru.",
  "profile.totpDisable": "Odpojit autentikátor",
  "profile.totpRegenerateRecoveryCodes": "Nové záložní kódy",
  "profile.totpRecoveryCodesRegenerated": "Záložní kódy byly obnoveny.",
  "profile.totpOperationFailed": "Požadavek na autentikátor selhal.",
  "profile.linkedAccounts": "Propojené účty",
  "profile.linkedAccountsDescription":
    "Spravujte identity Jellyfin propojené s tímto účtem Scryer.",
  "profile.linkedAccountsEmpty": "Zatím není propojen žádný externí účet.",
  "profile.linkedAccountsLoadFailed": "Načtení propojených účtů se nezdařilo.",
  "profile.linkAccountLoadFailed": "Načtení možností propojení se nezdařilo.",
  "profile.linkedAccountConnection": "Připojení",
  "profile.linkedAccountUnknownConnection": "Neznámé připojení",
  "profile.linkedAccountStatus": "Stav",
  "profile.linkJellyfinAccount": "Propojit s Jellyfin",
  "profile.linkPlexAccount": "Propojit s Plex",
  "profile.linkAccountConnection": "Připojení",
  "profile.linkAccountUsername": "Uživatelské jméno",
  "profile.linkAccountPassword": "Heslo",
  "profile.linkAccountPasswordlessHint":
    "Ponechte prázdné, pokud tento účet Jellyfin nemá heslo. Účty bez hesla lze propojit, ale nelze je použít k přihlášení do Scryeru.",
  "profile.linkAccountSubmit": "Propojit účet",
  "profile.linkAccountCancel": "Zrušit",
  "profile.linkAccountNoConnections":
    "Není k dispozici žádné vhodné připojení.",
  "profile.linkAccountLinked": "Externí účet propojen.",
  "profile.linkAccountFailed": "Propojení externího účtu se nezdařilo.",
  "profile.signInWithPlexToLink": "Přihlásit se přes Plex",
  "profile.unlinkAccount": "Odpojit",
  "profile.linkedAccountUnlinked": "Propojený účet odebrán.",
  "profile.linkedAccountUnlinkFailed": "Odpojení účtu se nezdařilo.",
  "auth.signIn": "Přihlásit se",
  "auth.signingIn": "Přihlašování…",
  "auth.username": "Uživatelské jméno",
  "auth.password": "Heslo",
  "auth.totpCode": "Jednorázový kód",
  "auth.signInFailedGeneric":
    "Přihlášení se nezdařilo. Zkontrolujte přihlašovací údaje a zkuste to znovu.",
  "auth.signInRateLimited":
    "Příliš mnoho pokusů o přihlášení. Počkejte chvíli a zkuste to znovu.",
  "auth.signInWithScryerPassword": "Přihlásit se heslem Scryer",
  "auth.signInWithPasskey": "Přihlásit se přístupovým klíčem",
  "auth.signInWithJellyfin": "Přihlásit se přes Jellyfin",
  "auth.jellyfinFailed": "Přihlášení přes Jellyfin se nezdařilo.",
  "auth.signInWithPlex": "Přihlásit se přes Plex",
  "auth.plexFailed": "Přihlášení přes Plex se nezdařilo.",
  "auth.plexPinFlowPending": "Dokončete přihlášení v okně Plex.",
  "auth.passkeySigningIn": "Přihlašování přístupovým klíčem…",
  "auth.passkeyUnsupported": "Tento prohlížeč nepodporuje přístupové klíče.",
  "auth.passkeyCancelled": "Požadavek na přístupový klíč byl zrušen.",
  "auth.totpCodeRequired":
    "Pro dokončení přihlášení zadejte jednorázový kód Scryer.",
  "auth.mfaSetupTitle": "Nastavení MFA Scryer",
  "auth.mfaSetupDescription":
    "Přihlášení proběhlo úspěšně. Nastavte si nyní jednorázové kódy Scryer, aby bylo možné ověřit budoucí přihlášení.",
  "auth.mfaSetupStartFailed": "Nastavení MFA se nepodařilo spustit.",
  "auth.mfaSetupCompleteFailed": "Nastavení MFA se nepodařilo dokončit.",
  "auth.mfaSetupCancel": "Zpět na přihlášení",
  "auth.mfaSetupRestart": "Spustit nastavení MFA znovu",
  "auth.mfaRecoveryCodesDescription":
    "Uložte si tyto záložní kódy na bezpečném místě. Každý kód lze použít jednou, pokud se nedostanete k autentikátoru.",
  "auth.continue": "Pokračovat",
  "auth.logoutButton": "Odhlásit se",
  "nav.wanted": "Požadované",
  "nav.requests": "Žádosti",
  "requests.empty": "Pro tento filtr nejsou žádné čekající žádosti.",
  "requests.emptyMine": "Pro tento filtr nejsou žádné žádosti.",
  "requests.mode.admin": "Fronta",
  "requests.mode.mine": "Moje",
  "requests.approve": "Schválit",
  "requests.approveTitle": "Schválit žádost",
  "requests.approvedQualityProfile": "Schválený profil kvality",
  "requests.approvedMonitorType": "Schválené sledování",
  "requests.dismiss": "Odmítnout",
  "requests.reopen": "Znovu otevřít",
  "requests.requesterFilter.label": "Žadatel",
  "requests.requesterFilter.all": "Všichni žadatelé",
  "status.requestReopened": "Žádost pro {{name}} byla znovu otevřena.",
  "requests.modify": "Upravit",
  "requests.modifyTitle": "Upravit žádost",
  "requests.saveChanges": "Uložit změny",
  "requests.cancelRequest": "Zrušit žádost",
  "requests.requesters": "Žadatelé",
  "requests.requestedQualityProfile": "Požadovaný profil kvality",
  "requests.requestedMonitorType": "Požadované sledování",
  "requests.libraryDefaultProfile": "Výchozí pro knihovnu",
  "requests.updated": "Aktualizováno",
  "requests.status.pending": "Čeká na vyřízení",
  "requests.status.approved": "Schváleno",
  "requests.status.canceled": "Zrušeno",
  "status.requestApproved": "{{name}} schváleno.",
  "status.requestApprovedSearchFailed":
    "{{name}} schváleno, ale vyhledávání požadovaného titulu nebylo zařazeno do fronty: {{error}}",
  "status.requestDismissed": "Žádost pro {{name}} byla odmítnuta.",
  "status.requestUpdated": "Žádost pro {{name}} byla aktualizována.",
  "status.requestCanceled": "Žádost pro {{name}} byla zrušena.",
  "wanted.title": "Požadované položky",
  "wanted.refreshing": "Obnovuje se…",
  "wanted.filterTitle": "Filtr podle názvu",
  "wanted.filterTitlePlaceholder": "Filtrovat podle názvu…",
  "wanted.totalCount": "Počet položek: {{count}}",
  "wanted.colTitle": "Název",
  "wanted.colType": "Typ",
  "wanted.colStatus": "Stav",
  "wanted.colConvergence": "Pokrytí",
  "wanted.colLastSearch": "Poslední hledání",
  "wanted.colIndexers": "Indexery",
  "wanted.convergence.searching": "Hledání {{covered}}/{{routed}}",
  "wanted.convergence.converged": "Sledování RSS",
  "wanted.convergence.convergedHint":
    "Prohledány všechny indexery — nová vydání přicházejí přes RSS. Spusťte hledání pro opětovnou kontrolu.",
  "wanted.convergence.deferred": "Čeká se na kvótu indexeru",
  "wanted.convergence.deferredHint":
    "Všechny zbývající indexery se ochlazují nebo mají vyčerpanou kvótu; hledání se automaticky obnoví.",
  "wanted.convergence.queuedHot": "Ve frontě · nedávné",
  "wanted.convergence.queuedCold": "Ve frontě · starší",
  "wanted.searchJobStarted": "Hledání zahájeno.",
  "wanted.searchJobProgress":
    "Dokončeno kroků hledání: {{processed}}/{{total}}. {{title}}",
  "wanted.searchNoEligibleWork":
    "Žádné sledované položky nevyžadují stažení ani upgrade.",
  "wanted.searchNoAcceptableReleases":
    "Hledání dokončeno; nenalezeno žádné vhodné vydání.",
  "wanted.searchSnapshotMissing":
    "Odpověď hledání byla neúplná. Před dalším pokusem zkontrolujte Aktivitu.",
  "wanted.searchInvalidSeason":
    "Tuto řadu se nepodařilo identifikovat. Obnovte titul a zkuste to znovu.",
  "wanted.searchJobComplete":
    "Hledání dokončeno. Prohledáno: {{processed}}, zařazeno ke stažení: {{grabbed}}, selhalo: {{failed}}.",
  "wanted.searchJobCancelled":
    "Hledání zrušeno. Zpracováno položek: {{processed}}; zařazeno ke stažení: {{grabbed}}.",
  "wanted.colPhase": "Fáze",
  "wanted.colLatestDecision": "Poslední rozhodnutí",
  "wanted.colScore": "Skóre",
  "wanted.searchNow": "Hledat nyní",
  "wanted.pause": "Pozastavit",
  "wanted.resume": "Pokračovat",
  "wanted.actionRecoverMismatch": "Opravit neshodu",
  "wanted.status.wanted": "Požadováno",
  "wanted.status.grabbed": "Zařazeno ke stažení",
  "wanted.status.completed": "Dokončeno",
  "wanted.status.paused": "Pozastaveno",
  "wanted.type.movie": "Film",
  "wanted.type.episode": "Epizoda",
  "wanted.type.seriesMovie": "Seriálový film",
  "wanted.decision.eligible": "Vyhovuje",
  "wanted.decision.titleMismatch": "Neshoda názvu",
  "wanted.decision.titleYearMismatch": "Jiný rok",
  "wanted.decision.episodicReleaseForMovie": "Epizody, ne film",
  "wanted.decision.episodeMismatch": "Neshoda epizody",
  "wanted.decision.categoryMismatch": "Neshoda kategorie",
  "wanted.decision.ambiguousIdentity": "Nejednoznačná identita",
  "wanted.decision.qualityBlocked": "Kvalita zablokována",
  "wanted.decision.minimumSeeders": "Málo seederů",
  "wanted.decision.packBelowMissingThreshold": "Balíček pod prahem chybějících",
  "wanted.decision.packForSingleEpisode": "Balíček pro jedinou epizodu",
  "wanted.decision.upgradeRejected": "Upgrade odmítnut",
  "wanted.decision.pendingDelay": "Čeká na zpoždění",
  "wanted.decision.alreadyActive": "Již aktivní",
  "wanted.decision.acceptInitial": "Přijmout počáteční",
  "wanted.decision.acceptUpgrade": "Přijmout upgrade",
  "wanted.decision.rejectInsufficientDelta": "Nedostatečný rozdíl",
  "wanted.decision.rejectCooldown": "Ochlazování",
  "wanted.decision.rejectNotAllowed": "Nepovoleno",
  "wanted.loadingDecisions": "Načítají se rozhodnutí o vydáních…",
  "wanted.noDecisions": "Zatím nebyla zaznamenána žádná rozhodnutí o vydáních.",
  "wanted.standby": "Pohotovost · {{count}}",
  "wanted.standbyTooltip":
    "Uložených kandidátů, kteří se vyzkouší, pokud aktuální stažení selže: {{count}}.",
  "wanted.standbyScopeNote":
    "Počty řádků se vážou na tento rozsah; balíčky řad se počítají u kotevní epizody řady.",
  "wanted.loadingStandby": "Načítají se uložení kandidáti…",
  "wanted.standbyCandidates": "Pohotovostní kandidáti · {{count}}",
  "wanted.standbyRank": "Pořadí {{rank}}",
  "wanted.standbyIndexer": "Indexer",
  "wanted.standbySize": "Velikost",
  "wanted.standbySeeders": "Seedeři",
  "wanted.standbyScore": "Skóre",
  "wanted.standbyAge": "Stáří",
  "wanted.seasonPack": "Balíček řady",
  "wanted.noStandbyCandidates": "žádní pohotovostní kandidáti",
  "wanted.noItems": "Nebyly nalezeny žádné požadované položky.",
  "wanted.context.movie": "Požadovaný film",
  "wanted.context.episode": "Požadovaná epizoda",
  "wanted.context.seasonEpisode": "Požadovaná epizoda řady {{seasonNumber}}",
  "wanted.context.seriesMovie": "Požadovaný seriálový film",
  "wanted.prev": "Předchozí",
  "wanted.next": "Další",
  "wanted.timeNow": "teď",
  "wanted.timeHoursMinutes": "{{hours}} h {{minutes}} min",
  "wanted.timeMinutes": "{{minutes}} min",
  "wanted.decRelease": "Vydání",
  "wanted.decDecision": "Rozhodnutí",
  "wanted.decScore": "Skóre",
  "wanted.decDelta": "Rozdíl",
  "wanted.decSize": "Velikost",
  "wanted.decDate": "Datum",
  "wanted.scoreBreakdown": "Rozpis hodnocení",
  "wanted.scoreCode": "Kód",
  "wanted.tabWanted": "Požadované položky",
  "wanted.tabCutoff": "Nesplněný cutoff",
  "cutoff.title": "Upgrady",
  "cutoff.searchAll": "Hledat vše",
  "cutoff.searchProgress": "Hledání {{current}} z {{total}}…",
  "cutoff.colTitleEpisode": "Název / epizoda",
  "cutoff.colCurrentQuality": "Aktuální",
  "cutoff.colTargetQuality": "Cílová",
  "cutoff.noItems":
    "Všechny sledované filmy a epizody splňují cutoff profilu kvality.",
  "cutoff.totalCount": "Položek pod cutoff: {{count}}",
  "cutoff.filterFacet": "Typ",
  "cutoff.allFacets": "Všechny typy",
  "cutoff.searchTriggered": "Vyhledání upgradu pro {{name}} bylo spuštěno",
  "settings.notifications": "Oznámení",
  "settings.notificationChannels": "Kanály",
  "settings.notificationSubscriptions": "Odběry",
  "settings.notificationChannelCreate": "Přidat kanál",
  "settings.notificationChannelCreateNew": "Vytvořit nový kanál",
  "settings.notificationChannelUpdate": "Aktualizovat kanál",
  "settings.notificationSubscriptionCreate": "Přidat odběr",
  "settings.notificationSubscriptionCreateNew": "Vytvořit nový odběr",
  "settings.notificationSubscriptionUpdate": "Aktualizovat odběr",
  "settings.notificationChannelConfirmDiscardTitle":
    "Zahodit aktuální úpravy kanálu?",
  "settings.notificationChannelConfirmDiscardDescription":
    "Vaše aktuální úpravy kanálu oznámení budou ztraceny.",
  "settings.notificationSubscriptionConfirmDiscardTitle":
    "Zahodit aktuální úpravy odběru?",
  "settings.notificationSubscriptionConfirmDiscardDescription":
    "Vaše aktuální úpravy odběru oznámení budou ztraceny.",
  "settings.notificationNoChannels":
    "Nejsou nakonfigurovány žádné kanály oznámení.",
  "settings.notificationNoSubscriptions":
    "Nejsou nakonfigurovány žádné odběry oznámení.",
  "settings.notificationEventType": "Typ události",
  "settings.notificationScope": "Rozsah",
  "settings.notificationScope.global": "Globální",
  "settings.notificationScope.facet": "Oblast",
  "settings.notificationScope.title": "Titul",
  "settings.notificationScopeId": "ID rozsahu",
  "settings.notificationScopeIdPlaceholderTitle": "ID titulu",
  "settings.notificationChannel": "Kanál",
  "settings.notificationTest": "Test",
  "settings.notificationTestSuccess":
    "Testovací oznámení bylo úspěšně odesláno.",
  "settings.notificationTestFailed": "Testovací oznámení se nezdařilo.",
  "settings.notificationProviderType": "Typ poskytovatele",
  "settings.notificationNoProviders":
    "Nejsou nainstalovány žádné pluginy pro oznámení. Nejdřív nainstalujte plugin pro oznámení na stránce Pluginy.",
  "settings.notificationJellyfinSubscriptionHint":
    "Import dokončen je obvyklý spouštěč skenování Jellyfin po importu. Zde jsou k dispozici pouze události Jellyfin, které mohou obnovit knihovnu.",
  "settings.notificationEvent.grab": "Vydání zařazeno ke stažení",
  "settings.notificationEvent.download": "Stahování selhalo",
  "settings.notificationEvent.upgrade": "Upgrade importován",
  "settings.notificationEvent.importComplete": "Import dokončen",
  "settings.notificationEvent.importRejected": "Import zablokován",
  "settings.notificationEvent.rename": "Soubor přejmenován",
  "settings.notificationEvent.titleAdded": "Titul přidán",
  "settings.notificationEvent.titleDeleted": "Titul smazán",
  "settings.notificationEvent.titleMoved": "Titul přesunut",
  "settings.notificationEvent.fileDeleted": "Soubor smazán",
  "settings.notificationEvent.fileDeletedForUpgrade":
    "Soubor smazán kvůli upgradu",
  "settings.notificationEvent.fileRestored": "Soubor obnoven",
  "settings.notificationEvent.postProcessingCompleted":
    "Dokončeno následné zpracování",
  "settings.notificationEvent.subtitleDownloaded": "Titulky staženy",
  "settings.notificationEvent.subtitleSearchFailed": "Hledání titulků selhalo",
  "settings.notificationEvent.mediaRequestSubmitted":
    "Žádost o médium odeslána",
  "settings.notificationEvent.mediaRequestApproved":
    "Žádost o médium schválena",
  "settings.notificationEvent.mediaRequestRejected":
    "Žádost o médium zamítnuta",
  "settings.notificationEvent.mediaRequestCanceled": "Žádost o médium zrušena",
  "settings.notificationEvent.healthIssue": "Problém se stavem systému",
  "settings.notificationEvent.healthRestored": "Stav systému obnoven",
  "settings.notificationEvent.applicationUpdate": "Aktualizace aplikace",
  "settings.notificationEvent.manualInteractionRequired":
    "Vyžadován ruční zásah",
  "settings.notificationEvent.test": "Testovací oznámení",
  "settings.notificationEvent.listTitleAdded": "Seznam přidal titul",
  "settings.notificationEvent.listRequestSubmitted": "Seznam odeslal žádost",
  "settings.notificationEvent.listItemHeld": "Položka seznamu pozdržena",
  "settings.notificationEvent.listTitleLeft": "Titul opustil seznam",
  "settings.notificationEvent.listSyncFailed": "Synchronizace seznamu selhala",
  "settings.notificationEvent.listUnfollowed": "Sledování seznamu zrušeno",
  "status.notificationChannelCreated": "Kanál oznámení uložen.",
  "status.notificationChannelUpdated": "Kanál oznámení aktualizován.",
  "status.notificationChannelDeleted": "Kanál oznámení {{name}} smazán.",
  "status.notificationSubscriptionCreated": "Odběr oznámení uložen.",
  "status.notificationSubscriptionUpdated": "Odběr oznámení aktualizován.",
  "status.notificationSubscriptionDeleted": "Odběr oznámení smazán.",
  "status.editingNotificationChannel": "Úprava kanálu {{name}}",
  "status.deletingNotificationChannel": "Smazat kanál oznámení {{name}}?",
  "status.editingNotificationSubscription": "Úprava odběru",
  "status.deletingNotificationSubscription": "Smazat tento odběr oznámení?",
  "wanted.tabPending": "Čekající",
  "pending.title": "Čekající vydání",
  "pending.noItems": "Žádná čekající vydání.",
  "pending.colRelease": "Vydání",
  "pending.colScore": "Skóre",
  "pending.colSize": "Velikost",
  "pending.colIndexer": "Indexer",
  "pending.colAddedAt": "Přidáno",
  "pending.colDelayUntil": "Dostupné",
  "pending.colReason": "Důvod",
  "pending.colRole": "Role",
  "pending.role.primary": "Primární",
  "pending.role.fallback": "Náhradní",
  "pending.status.needsReview": "Vyžaduje kontrolu",
  "pending.phase.needsReview": "Kontrola",
  "pending.forceGrab": "Stáhnout nyní",
  "pending.dismiss": "Odmítnout",
  "pending.grabbed": "Vydání zařazeno ke stažení.",
  "pending.grabRejected":
    "Stažení odmítnuto — vydání je zablokované nebo nedostupné.",
  "pending.dismissed": "Vydání odmítnuto.",
  "setup.welcomeTitle": "Vítejte ve Scryeru",
  "setup.welcomeDescription":
    "Pojďme nastavit to nejdůležitější, abyste mohli začít.",
  "setup.freshSetup": "Nastavit od začátku",
  "setup.freshSetupDescription":
    "Nastavte mediální cesty, nainstalujte poskytovatele jako qBittorrent a postupně přidejte klienty a indexery.",
  "setup.importSetup": "Importovat ze Sonarru / Radarru",
  "setup.importSetupDescription":
    "Přeneste svou stávající konfiguraci do Scryeru.",
  "setup.restoreSetup": "Obnovit ze zálohy",
  "setup.restoreSetupDescription":
    "Nahrajte balíček zálohy Scryeru a obnovte tuto novou instanci.",
  "setup.stepPersona": "Persona",
  "setup.stepMediaPaths": "Mediální cesty",
  "setup.stepPlugins": "Pluginy",
  "setup.stepDownloadClient": "Klient pro stahování",
  "setup.stepIndexer": "Indexer",
  "setup.stepSummary": "Shrnutí",
  "setup.stepConnect": "Připojit",
  "setup.stepRestore": "Obnovit",
  "setup.restoreTitle": "Obnovit z balíčku zálohy",
  "setup.restoreDescription":
    "Nahrajte balíček zálohy Scryeru, prohlédněte si jeho manifest a obnovte jej do této nové instance.",
  "setup.restoreSelectBundle": "Balíček zálohy",
  "setup.restoreDropTargetTitle": "Přetáhněte sem balíček zálohy",
  "setup.restoreDropTargetDescription":
    "Vyberte soubor zálohy ze svého zařízení nebo jej sem přetáhněte pro prohlédnutí před obnovením.",
  "setup.restoreDropTargetSelected":
    "Balíček vybrán a připraven k prohlédnutí.",
  "setup.restoreDropTargetEncryptedSelected":
    "Vybrán šifrovaný balíček. Pro pokračování zadejte heslo použité při vytvoření zálohy.",
  "setup.restoreDropTargetFormats": "Podporované formáty: .tar.zst a .enc",
  "setup.restoreSelectFile": "Vybrat soubor",
  "setup.restoreClearFile": "Zrušit výběr",
  "setup.restorePasswordHelp":
    "Zadejte heslo použité při vytvoření této šifrované zálohy.",
  "setup.restoreNoFile": "Pro pokračování vyberte balíček zálohy.",
  "setup.restorePasswordRequired": "Před pokračováním zadejte heslo balíčku.",
  "setup.restoreInspect": "Prohlédnout balíček",
  "setup.restoreApply": "Obnovit nyní",
  "setup.restoreChooseAnother": "Vybrat jiný balíček",
  "setup.restoreSummaryTitle": "Obsah balíčku",
  "setup.restoreConfirmDescription":
    "Tím se nahradí databáze nového nastavení, zapíší se obnovené tajné klíče instance a Scryer se restartuje do obnovené instance.",
  "setup.restoreCreatedAt": "Vytvořeno",
  "setup.restoreSourceVersion": "Zdrojová verze",
  "setup.restoreSourceEngine": "Zdrojový engine",
  "setup.restoreMigrationKey": "Zdrojový migrační klíč",
  "setup.restoreTotalRows": "Celkem řádků",
  "setup.personaTitle": "Preference kvality",
  "setup.personaDescription":
    "Zvolte cílovou kvalitu a zaměření hodnocení pro každý typ obsahu.",
  "setup.facetMovies": "Filmy",
  "setup.facetSeries": "Seriály",
  "setup.facetAnime": "Anime",
  "setup.qualityTarget": "Kvalita",
  "setup.scoringFocus": "Persona",
  "setup.personaBalancedDesc":
    "Rozumné výchozí hodnoty pro většinu případů. Dobrá kvalita bez honby za remux vydáními.",
  "setup.personaAudiophileDesc":
    "Maximální věrnost. Upřednostňuje BluRay, remux, bezeztrátový zvuk, HDR a velké soubory.",
  "setup.personaEfficientDesc":
    "Nejlepší kvalita na GB. Preferuje x265, WEB-DL, kompaktní enkódy.",
  "setup.personaCompatibleDesc":
    "Přehraje se všude. H.264 před H.265, vyhýbá se Dolby Vision, široká podpora zařízení.",
  "setup.pluginsTitle": "Nainstalujte poskytovatele, které chcete používat",
  "setup.pluginsDescription":
    "Některé služby, jako qBittorrent, se před nakonfigurováním ve Scryeru přidávají jako pluginy.",
  "setup.pluginsBuiltInTitle": "Již součástí",
  "setup.pluginsBuiltInDescription":
    "NZBGet a SABnzbd jsou připraveny ke konfiguraci bez instalace čehokoli navíc.",
  "setup.pluginsAvailableHeading": "Oficiální pluginy",
  "setup.pluginsAvailableHint":
    "Nainstalujte oficiální pluginy, které na tomto hostiteli plánujete používat. Později je stále můžete změnit v Nastavení.",
  "setup.pluginsNoneFound":
    "Nebyly nalezeny žádné pluginy. Obnovte registr nebo pokračujte se vestavěnými poskytovateli.",
  "setup.recommendedHeading": "Doporučeno",
  "setup.recommendedHint":
    "Pluginy, které potřebuje většina instalací. Nainstalujte ty, které odpovídají vašemu způsobu používání Scryer.",
  "setup.recommendedArchiveTitle": "Rozbalování archivů",
  "setup.recommendedArchiveReason":
    "Nainstalujte, pokud používáte torrenty nebo stahujete titulky. Oboje často přichází zabaleno v archivech RAR nebo ZIP.",
  "setup.recommendedQbittorrentTitle": "qBittorrent",
  "setup.recommendedSubtitlesTitle": "Titulky",
  "setup.recommendedSubtitlesReason":
    "Nainstalujte, pokud Scryer spravuje vaše titulky. Enhanced Sync sesynchronizuje titulky s videem, OpenSubtitles pokrývá filmy a seriály a Jimaku s AnimeTosho pokrývají anime.",
  "setup.recommendedAdvancedIndexersTitle": "Pokročilé indexery",
  "setup.recommendedAdvancedIndexersReason":
    "Nainstalujte, pokud používáte složitější indexery Prowlarr založené na jeho enginu Cardigann.",
  "setup.recommendedMediaServerTitle": "Mediální server",
  "setup.recommendedMediaServerReason":
    "Obnoví váš mediální server, když Scryer něco importuje. Vyberte ten, který používáte.",
  "setup.rulePacksHeading": "Sady pravidel",
  "setup.rulePacksHint":
    "Komunitní pravidla hodnocení, která pomáhají Scryer vybrat nejlepší vydání. Později je můžete upravit v Nastavení → Pravidla.",
  "setup.rulePackTrashTitle": "TRaSH Guides",
  "setup.rulePackTrashReason":
    "Hodnotí vydání podle doporučení TRaSH Guides pro release skupiny, zvuk, HDR, velikost a nežádoucí vydání.",
  "setup.rulePackSeadexTitle": "SeaDex",
  "setup.rulePackSeadexReason":
    "Zapněte, pokud sbíráte anime. Upřednostňuje vydání, která SeaDex označuje jako nejlepší dostupná.",
  "setup.rulePackUnavailable":
    "Momentálně není k dispozici. Obnovte registr a zkuste to znovu.",
  "setup.otherPluginsHeading": "Další pluginy",
  "setup.mediaPathsTitle": "Kde jsou uložena vaše média?",
  "setup.mediaPathsDescription":
    "Nastavte kořenové složky, ve kterých Scryer bude organizovat vaše filmy a seriály.",
  "setup.moviesPath": "Cesta k filmům",
  "setup.seriesPath": "Cesta k seriálům",
  "setup.animePath": "Cesta k anime",
  "setup.optional": "(volitelné)",
  "setup.browse": "Procházet",
  "setup.addMediaFolder": "Přidat složku",
  "setup.removeMediaFolder": "Odebrat složku",
  "setup.mediaPathsNoneChosen":
    "Nebyly vybrány žádné složky, tato knihovna si tedy ponechá stávající složky.",
  "setup.mediaPathNotReachable": "Zatím nedostupná",
  "setup.mediaPathsNotReachableWarning":
    "Některé cesty k médiím nejsou zatím dostupné. Byly uloženy a lze je později změnit v Nastavení.",
  "setup.mediaPathsVerificationUnavailable":
    "Tyto cesty se nepodařilo ověřit. Můžete pokračovat a změnit je později v Nastavení.",
  "setup.downloadClientTitle": "Přidat klienta pro stahování",
  "setup.downloadClientDescription":
    "Scryer odesílá stahování do vašeho preferovaného klienta. Pluginy nainstalované v předchozím kroku se zobrazí zde.",
  "setup.indexerTitle": "Přidat indexer",
  "setup.indexerDescription":
    "Indexery poskytují databázi vydání, ve které Scryer vyhledává.",
  "setup.summaryTitle": "Vše je nastaveno",
  "setup.summaryDescription":
    "Zde je přehled toho, co jste nastavili. Kteroukoli z těchto věcí můžete později změnit v Nastavení.",
  "setup.summaryPersona": "Persona",
  "setup.summaryMediaPaths": "Cesty k médiím",
  "setup.summaryDownloadClient": "Klient pro stahování",
  "setup.summaryIndexer": "Indexer",
  "setup.next": "Další",
  "setup.back": "Zpět",
  "setup.finish": "Dokončit nastavení",
  "setup.importOnly": "Pouze import",
  "setup.importAndScan": "Import a skenování",
  "setup.connectionSuccess": "Připojení úspěšné",
  "setup.connectionFailed": "Připojení se nezdařilo",
  "setup.saved": "Uloženo",
  "setup.required": "Toto pole je povinné",
  "setup.skip": "Přeskočit",
  "setup.skipSetup": "Úplně přeskočit nastavení",
  "setup.skipping": "Přeskakuje se…",
  "setup.findApiKey": "Najít svůj klíč API",
  "setup.connectAndScan": "Připojit a skenovat",
  "setup.connectError": "Připojení se nezdařilo",
  "setup.connected": "připojeno",
  "setup.notSupported": "Nepodporováno",
  "setup.importing": "Importuje se…",
  "setup.summaryImportedClients": "Importovaní klienti pro stahování",
  "setup.summaryImportedIndexers": "Importované indexery",
  "setup.monitorWarmupTitle": "Příprava importu stavu sledování",
  "setup.monitorWarmupDescription":
    "Scryer na pozadí stahuje stav sledování, aby byl závěrečný krok importu rychlejší.",
  "setup.monitorWarmupQueued": "Čeká se na podrobnosti o průběhu…",
  "setup.monitorWarmupLoadingIndexers": "Zjišťování indexerů",
  "setup.monitorWarmupLoadingMovies": "Načítání filmů",
  "setup.monitorWarmupLoadingSeries": "Načítání seriálů",
  "setup.monitorWarmupLoadingEpisodes": "Načítání epizod",
  "setup.monitorWarmupBuildingSnapshot": "Vytváření snímku stavu",
  "setup.monitorWarmupReady": "Připraveno",
  "setup.monitorWarmupFailed":
    "Při přípravě na pozadí došlo k chybě. Scryer to zkusí znovu, až spustíte import.",
  "setup.monitorWarmupCanceled":
    "Příprava na pozadí byla zrušena. Scryer ji v případě potřeby restartuje.",
  "setup.importFinalizeFailed":
    "Scryer nedokázal dokončit přípravu stavu sledování pro tento import.",
  "setup.apiKeyMasked":
    "Sonarr/Radarr tento klíč API skrývá. Pro pokračování importu ho zadejte ručně:",
  "setup.stepLibraries": "Knihovny",
  "setup.stepQuality": "Kvalita",
  "setup.stepSources": "Zdroje",
  "setup.continue": "Pokračovat",
  "setup.finishImport": "Dokončit import",
  "setup.connectMultiTitle": "Připojení k Sonarru, Radarru a Prowlarru",
  "setup.connectMultiDescription":
    "Přidejte URL a klíč API pro každou instanci, ze které má Scryer stahovat data. Můžete připojit více instancí od každého — alespoň jedna je povinná.",
  "setup.librariesTitle": "Knihovny a mapování",
  "setup.librariesDescription":
    "Vytvořte knihovnu pro každý způsob, jakým chcete obsah třídit, a poté do ní namapujte každý zdrojový kořen.",
  "setup.qualityPersonaTitle": "Kvalita a persona",
  "setup.qualityPersonaDescription":
    "Pro každou namapovanou knihovnu vyberte profil kvality a ladicí personu.",
  "setup.sourcesTitle": "Klienti a indexery",
  "setup.sourcesDescription":
    "Vyberte, které klienty pro stahování a indexery chcete převzít.",
  "setup.importReadyTitle": "Připraveno k importu",
  "setup.importReadyDescription": "Než Scryer vše načte, vše zkontrolujte.",
  "setup.addInstance": "Přidat {{product}}",
  "setup.instanceName": "Název instance",
  "setup.unnamedInstance": "Nepojmenovaná instance",
  "setup.removeInstance": "Odebrat instanci",
  "setup.testing": "Testuje se…",
  "setup.couldntConnect": "Nepodařilo se připojit",
  "setup.enterUrlAndKey": "Zadejte URL a klíč API k ověření",
  "setup.apiKeyHelpHint": "Nastavení → Obecné",
  "setup.secretDraftOtherUserWarning":
    "Jiný uživatel má uložený koncept importu. Zadáním vlastních klíčů zde nahradíte jeho uložené přihlašovací údaje.",
  "setup.sourceRoots": "Zdrojové kořeny",
  "setup.addSourceRoot": "Přidat zdrojový kořen",
  "setup.chooseFolder": "Vybrat složku…",
  "setup.unmappedCountOne": "1 nenamapovaný",
  "setup.unmappedCountMany": "Nenamapováno: {{count}}",
  "setup.allRootsMapped":
    "Všechny zdrojové kořeny jsou namapovány — jeden sem přetáhněte zpět, chcete-li mapování zrušit",
  "setup.dragRootIntoLibrary": "Přetáhněte každý kořen do knihovny",
  "setup.dropRootsHere": "Kořeny upusťte zde",
  "setup.removeSourceRoot": "Odebrat zdrojový kořen",
  "setup.manualRootPathAria": "Ruční cesta zdrojového kořene",
  "setup.renameLibraryAria": "Přejmenovat {{name}}",
  "setup.previewLoading": "Načítání zdrojových kořenů z vašich instancí…",
  "setup.previewError": "Zdrojové kořeny se nepodařilo načíst.",
  "setup.retry": "Zkusit znovu",
  "setup.noCompatibleLibrary": "Zatím žádná kompatibilní knihovna.",
  "setup.createFacetLibrary": "Vytvořit knihovnu pro oblast {{facet}}",
  "setup.addLibrary": "Přidat knihovnu",
  "setup.newLibraryFor": "Nová knihovna pro…",
  "setup.cancel": "Zrušit",
  "setup.renameLibrary": "Přejmenovat",
  "setup.renameInstance": "Přejmenovat",
  "setup.deleteLibrary": "Smazat knihovnu",
  "setup.remap": "Přemapovat",
  "setup.assignToLibrary": "Přiřadit ke knihovně",
  "setup.moveToAnotherLibrary": "Přesunout do jiné knihovny",
  "setup.assignSheetTitle": "Přiřadit ke knihovně",
  "setup.moveToUnassigned": "Přesunout mezi nepřiřazené",
  "setup.noPathSet": "(cesta nenastavena)",
  "setup.remapTitle": "Přemapovat cestu",
  "setup.remapExplain":
    "Scryer si ukládá cestu, kterou hlásí {{instance}}, jako původ, aby mohl správně znovu použít stav sledování. Pokud se tyto soubory na hostiteli Scryeru nacházejí na jiné cestě, nastavte ji níže.",
  "setup.remapAsReported": "Jak hlásí zdroj",
  "setup.remapScryerHostPath": "Cesta na hostiteli Scryeru",
  "setup.remapResetToSource": "Obnovit na zdrojovou",
  "setup.remapSave": "Uložit přemapování",
  "setup.mappedPathValidationChecking":
    "Kontroluje se, zda má Scryer přístup ke každé namapované zdrojové cestě…",
  "setup.invalidMappedPathTitle":
    "Scryer nemá přístup k této namapované zdrojové cestě",
  "setup.invalidMappedPathsTitle":
    "Namapované zdrojové cesty, ke kterým Scryer nemá přístup: {{count}}",
  "setup.invalidMappedPathsHelp":
    "Tyto kořeny jsou přiřazené, ale jejich cesty nejsou pro Scryer viditelné. Než budete pokračovat, přemapujte každý z nich na odpovídající složku na hostiteli Scryeru.",
  "setup.remapInvalidRootAria": "Přemapovat {{name}} z {{path}}",
  "setup.library": "Knihovna",
  "setup.qualityProfile": "Profil kvality",
  "setup.persona": "Persona",
  "setup.qualityNoLibraries":
    "Namapujte alespoň jeden zdrojový kořen na knihovnu, abyste mohli nastavit její kvalitu.",
  "setup.persona.balanced.name": "Vyvážená",
  "setup.persona.balanced.desc": "Solidní kvalita při rozumné velikosti",
  "setup.persona.audiophile.name": "Audiofilní",
  "setup.persona.audiophile.desc": "Maximální věrnost, větší soubory",
  "setup.persona.efficient.name": "Úsporná",
  "setup.persona.efficient.desc": "Menší kódování, úspora místa na disku",
  "setup.persona.compatible.name": "Kompatibilní",
  "setup.persona.compatible.desc":
    "Vhodná pro přímé přehrávání, široká podpora",
  "setup.downloadClients": "Klienti pro stahování",
  "setup.indexers": "Indexery",
  "setup.merged": "SLOUČENO",
  "setup.from": "Z",
  "setup.passwordRequired": "Heslo je povinné.",
  "setup.indexerApiKeyHelpLink": "Najít klíč API vašeho indexeru",
  "setup.provenanceSource": "Zdroj",
  "setup.provenanceScryer": "Scryer",
  "setup.sourcesLoading": "Načítání klientů a indexerů…",
  "setup.duplicatesMerged": "Duplicity sloučeny napříč instancemi",
  "setup.summaryLibraries": "Knihovny",
  "setup.summaryLibrariesDetail": "Napříč filmy, seriály a anime",
  "setup.summaryInstancesConnected": "Připojené instance",
  "setup.summaryInstancesConnectedDetail":
    "{{sonarr}} Sonarr · {{radarr}} Radarr",
  "setup.summaryRootsMapped": "Namapované zdrojové kořeny",
  "setup.summaryRootsMappedDetail": "Přesměrováno do knihovny Scryer",
  "setup.summaryPathsRemapped": "Přemapované cesty",
  "setup.summaryPathsRemappedDetail": "Zachován původ pro stav sledování",
  "setup.summaryClientsMerged": "Sloučeno napříč instancemi",
  "setup.summaryIndexersEnabled": "Podporováno a zapnuto",
  "setup.fetchingMonitoredStatus": "Načítání stavu sledování…",
  "setup.fetchingMonitoredStatusDetail":
    "Na pozadí se stahuje stav sledování každého titulu z vašich instancí.",
  "setup.monitoredStatusSynced": "Stav sledování synchronizován",
  "setup.monitoredStatusSyncedDetail":
    "Stav sledování každého titulu byl stažen z vašich instancí.",
  "setup.importGateHint":
    "Import nelze dokončit, dokud se nedokončí synchronizace stavu sledování.",
  "setup.importGateDoneHint": "Hotovo — import teď můžete dokončit.",
  "setup.titlesFetched": "Tituly: {{fetched}} / {{total}}",
  "setup.importWarmupFailedTitle": "Synchronizace stavu sledování se nezdařila",
  "setup.importWarmupFailedDetail":
    "Scryeru se nepodařilo dokončit stažení stavu sledování z vašich instancí. Zkontrolujte připojení a zkuste to znovu.",
  "setup.importWarmupRetrying": "Restartování synchronizace…",
  "setup.importWarmupSessionExpiredTitle": "Platnost přípravy importu vypršela",
  "setup.importWarmupSessionExpired":
    "Vaše příprava importu už není dostupná — Scryer se mohl restartovat, nebo zůstala příliš dlouho nečinná. Pro nový import se znovu připojte ke svým instancím.",
  "setup.importReconnect": "Připojit znovu",
  "setup.applyingImport": "Použití importu…",
  "setup.applyingImportDetail":
    "Zapisuje se importovaný stav sledování do vašich knihoven. Pokud opustíte stránku, pokračuje to dál.",
  "setup.importApplied": "Import použit",
  "setup.importAppliedDetail":
    "Stav sledování všech importovaných titulů byl zapsán do vašich knihoven.",
  "setup.importFinalizeFailedTitle": "Dokončení importu se nezdařilo",
  "setup.titlesApplied": "Tituly: {{applied}} / {{total}}",
  "setup.importUnmappedRootsNotice":
    "Některé nalezené zdrojové složky stále potřebují knihovnu. Před dokončením se vraťte na krok Knihovny a namapujte je.",
  "settings.runSetupWizard": "Spustit průvodce nastavením",
  "history.title": "Historie",
  "history.empty": "Žádné události v historii",
  "history.loadError": "Historii se nepodařilo načíst.",
  "history.retry": "Zkusit znovu",
  "history.event": "Událost",
  "history.initial": "Počáteční",
  "history.upgrade": "Upgrade",
  "history.prospectiveEpisodes": "Očekávané epizody",
  "history.titleColumn": "Titul",
  "history.sourceTitle": "Zdrojový titul",
  "history.quality": "Kvalita",
  "history.date": "Datum",
  "history.actor": "Aktér",
  "history.allEvents": "Všechny události",
  "history.eventTypes": "Typy událostí",
  "history.eventTypesSelected": "Vybráno typů událostí: {{count}}",
  "history.grabbed": "Zařazeno ke stažení",
  "history.downloadFailed": "Stahování se nezdařilo",
  "history.blocklisted": "Na černé listině",
  "history.scanned": "Prohledáno",
  "history.imported": "Importováno",
  "history.importFailed": "Import se nezdařil",
  "history.importSkipped": "Import přeskočen",
  "history.importRejectedByRule": "Odmítnuto pravidlem",
  "history.downloadCompleted": "Staženo",
  "history.fileUpgraded": "Upgradováno",
  "history.fileRecycled": "Přesunuto do koše",
  "history.fileDeleted": "Smazáno",
  "history.fileRestored": "Obnoveno",
  "history.fileRenamed": "Přejmenováno",
  "history.titleMoved": "Přesunuto",
  "history.rematched": "Přiřazeno znovu",
  "history.seedingStarted": "Seedování",
  "history.seedingCompleted": "Seedování dokončeno",
  "history.downloadIgnored": "Ignorováno",
  "history.facet": "Oblast",
  "history.actions": "Akce",
  "history.expandDetails": "Rozbalit podrobnosti události",
  "history.collapseDetails": "Sbalit podrobnosti události",
  "history.collectionId": "ID kolekce",
  "history.episodeCountSingle": "1 epizoda",
  "history.episodeCountMultiple": "Epizod: {{count}}",
  "history.unknownEvent": "Neznámá",
  "history.noMore": "Žádné další události",
  "history.loadMore": "Načíst další",
  "indexerErrors.title": "Historie chyb indexeru",
  "indexerErrors.scopedTitle": "{{indexer}} — historie chyb",
  "indexerErrors.history": "Historie chyb",
  "indexerErrors.refresh": "Obnovit",
  "indexerErrors.retry": "Zkusit znovu",
  "indexerErrors.loading": "Načítání chyb indexeru…",
  "indexerErrors.loadingResponse": "Načítání zachycené odpovědi…",
  "indexerErrors.loadMore": "Načíst další",
  "indexerErrors.noMore": "Žádné další zachycené odpovědi",
  "indexerErrors.loadError": "Historii chyb indexeru se nepodařilo načíst.",
  "indexerErrors.detailLoadError": "Zachycenou odpověď se nepodařilo načíst.",
  "indexerErrors.detailMissing": "Tato zachycená odpověď už není dostupná.",
  "indexerErrors.empty":
    "Za posledních 30 dní nebyly zaznamenány žádné chyby indexeru.",
  "indexerErrors.retentionNotice":
    "Chyby indexeru se uchovávají 30 dní. HTTP odpovědi zahrnují zachycené hlavičky a tělo; u chyb přenosu se uchovávají pouze diagnostická data.",
  "indexerErrors.occurred": "Nastala",
  "indexerErrors.indexer": "Indexer",
  "indexerErrors.operation": "Operace",
  "indexerErrors.http": "HTTP",
  "indexerErrors.classification": "Klasifikace",
  "indexerErrors.message": "Zpráva",
  "indexerErrors.contentType": "Typ obsahu",
  "indexerErrors.providerCode": "Kód poskytovatele {{code}}",
  "indexerErrors.expand": "Rozbalit zachycenou odpověď",
  "indexerErrors.collapse": "Sbalit zachycenou odpověď",
  "indexerErrors.response": "Zachycená HTTP odpověď",
  "indexerErrors.noHttpResponse":
    "Nebyla přijata žádná HTTP odpověď; tato chyba přenosu má pouze diagnostické podrobnosti.",
  "indexerErrors.headers": "Hlavičky odpovědi",
  "indexerErrors.noHeaders": "Nebyly zachyceny žádné hlavičky odpovědi.",
  "indexerErrors.body": "Tělo odpovědi",
  "indexerErrors.formatted": "Formátováno",
  "indexerErrors.raw": "Surová data",
  "indexerErrors.redacted": "[před uložením odstraněno]",
  "indexerErrors.base64Value": "Base64",
  "indexerErrors.previewTruncated":
    "Náhled je omezen na prvních 1 MiB z {{size}}. Stáhněte celé tělo, chcete-li prohlédnout všechna zachycená data.",
  "indexerErrors.emptyBody": "Tělo odpovědi je prázdné.",
  "indexerErrors.binaryBody":
    "Tato odpověď o velikosti {{size}} není platné UTF-8. Stáhněte ji, chcete-li prohlédnout zachycená data.",
  "indexerErrors.downloadFullBody": "Stáhnout celé tělo",
  "settings.autoBackupsTitle": "Automatické zálohy",
  "settings.autoBackupsDescription":
    "Zapněte, pokud chcete jednu systémovou zálohu denně v čase podle serveru. Automatické zálohy mají časový limit 30 minut a Scryer uchovává posledních 5 úspěšných automatických záloh.",
  "settings.autoBackupsDisabledMissingKeyToastTitle":
    "Automatické zálohy byly vypnuty",
  "settings.autoBackupsDisabledMissingKeyToastDescription":
    "Automatické zálohy nyní vyžadují uložený záložní klíč. Otevřete Zálohy a nastavte klíč, abyste je znovu zapnuli.",
  "settings.autoBackupsDisabledMissingKeyToastAction": "Otevřít Zálohy",
  "settings.autoBackupsTime": "Čas denní zálohy",
  "settings.autoBackupsHour": "Hodina",
  "settings.autoBackupsMinute": "Minuta",
  "settings.autoBackupsTimeHelp": "Používá místní časové pásmo serveru.",
  "settings.autoBackupsNextRun": "Příští automatická záloha",
  "settings.autoBackupsNextRunHelp":
    "Uložením změn aktualizujete plánovač. Vypnuté automatické zálohy se nespouští.",
  "settings.autoBackupsKeyLabel": "Klíč automatické zálohy",
  "settings.autoBackupsKeyHelpLabel": "O klíči automatické zálohy",
  "settings.autoBackupsSetKey": "Nastavit klíč automatické zálohy",
  "settings.autoBackupsKeyAlreadySetHint": "(záložní klíč je již nastaven)",
  "settings.autoBackupsClearKey": "Vymazat uložený klíč automatické zálohy",
  "settings.autoBackupsClearKeyHelp":
    "Po uložení odstraní uložený klíč. Před vymazáním klíče vypněte automatické zálohy.",
  "settings.autoBackupsKeyHelp":
    "Pole nechte prázdné, chcete-li ponechat stávající uložený klíč. Pro jeho nahrazení zadejte novou hodnotu. Minimální počet znaků kromě mezer: {{count}}.",
  "settings.autoBackupsKeyRequired":
    "Automatické zálohy vyžadují uložený záložní klíč. Minimální počet znaků kromě mezer: {{count}}.",
  "settings.autoBackupsKeyTooShort":
    "Klíč automatických záloh je příliš krátký. Minimální počet znaků kromě mezer: {{count}}.",
  "settings.autoBackupsShowKey": "Zobrazit klíč automatické zálohy",
  "settings.autoBackupsHideKey": "Skrýt klíč automatické zálohy",
  "settings.autoBackupsSaved": "Nastavení automatických záloh uloženo.",
  "settings.backupLocationTitle": "Umístění záloh",
  "settings.backupLocationDescription":
    "Vyberte, kam bude Scryer na serveru ukládat záložní balíčky.",
  "settings.backupLocationCustomPath": "Vlastní cesta k zálohám",
  "settings.backupLocationBrowse": "Procházet",
  "settings.backupLocationEffectivePath": "Aktivní cesta",
  "settings.backupLocationHelp":
    "Použijte cestu na serveru nebo v kontejneru. Změna přepne aktivní záložní složku; stávající zálohy se nepřesunou a budou skryté, dokud se nevrátíte zpět.",
  "settings.backupLocationReset": "Použít výchozí",
  "settings.backupLocationPickerTitle": "Vybrat záložní složku",
  "settings.backupLocationSaved": "Umístění záloh uloženo.",
  "settings.backupsSection":
    "Vytvářejte obnovitelné záložní balíčky Scryer. Ruční zálohy běží na pozadí, takže můžete pokračovat v práci, zatímco se balíček připravuje, i když už běží automatická záloha.",
  "settings.backupsEmpty": "Zatím nebyly vytvořeny žádné zálohy.",
  "settings.backupsCreate": "Vytvořit zálohu",
  "settings.backupsCreateTitle": "Vytvořit záložní balíček",
  "settings.backupsCreateDescription":
    "Chraňte tuto zálohu heslem. Stejné heslo bude vyžadováno při obnově.",
  "settings.backupsRequiredPassword": "Povinná ochrana heslem",
  "settings.backupsConfirmPassword": "Potvrdit heslo",
  "settings.backupsPasswordMismatch": "Hesla se neshodují",
  "settings.backupsPasswordHelp":
    "Úplné záložní balíčky obsahují tajné údaje instancí a musí být zašifrovány.",
  "settings.backupsQueued": "Vytváření zálohy bylo spuštěno na pozadí.",
  "settings.backupsDeleted": "Záloha smazána.",
  "settings.backupsDeletedCount": "Smazáno záloh: {{count}}.",
  "settings.backupsDeleteFailedCount": "Nepodařilo se smazat záloh: {{count}}.",
  "settings.backupsCreating": "Vytváří se",
  "settings.backupsReady": "Připraveno",
  "settings.backupsInvalid": "Neplatné",
  "settings.backupsFailed": "Selhalo",
  "settings.backupsEncrypted": "Zašifrováno",
  "settings.backupsPlaintext": "Nešifrováno",
  "settings.backupsManual": "Ruční",
  "settings.backupsAutomatic": "Automatická",
  "settings.backupsDownload": "Stáhnout",
  "settings.backupsDelete": "Smazat",
  "settings.backupsDeleteConfirm":
    "Smazat tento záložní balíček ze serveru? Žádná stažená kopie nebude ovlivněna.",
  "settings.backupsDeleteSelected": "Smazat vybrané",
  "settings.backupsDeleteSelectedConfirm":
    "Smazat vybrané záložní balíčky ze serveru ({{count}})? Žádné stažené kopie nebudou ovlivněny.",
  "settings.backupsSelectedCount": "Vybráno: {{count}}",
  "settings.backupsSelect": "Vybrat {{name}}",
  "settings.backupsSelectAll": "Vybrat všechny zálohy",
  "settings.unsavedBackupChangesTitle": "Neuložené nastavení záloh",
  "settings.unsavedBackupChangesConfirm":
    "Máte neuložené změny v nastavení záloh. Opustit bez uložení?",
  "settings.recycleBin": "Koš",
  "settings.recycleBinEnabled": "Zapnout koš",
  "settings.recycleBinEnabledHelp":
    "Smazané a nahrazené mediální soubory se před trvalým odstraněním přesunou do koše.",
  "settings.recycleBinEnabledReadonly":
    "Stav koše řídí uživatelé, kteří mohou spravovat konfiguraci.",
  "settings.recycleBinPath": "Složka koše",
  "settings.recycleBinPathPlaceholder":
    ".scryer-recycle v každém kořeni knihovny",
  "settings.recycleBinPathHelp":
    "Ponechte prázdné pro použití složky .scryer-recycle uvnitř každého kořene knihovny. Vlastní složka musí být absolutní cesta mimo každý kořen knihovny.",
  "settings.recycleBinPathFilesystemWarning":
    "Udržujte složku na stejném souborovém systému jako vaše média. Na jiném souborovém systému recyklace každý soubor zkopíruje a poté smaže originál, což je pomalejší a ruší pevné odkazy (hardlinks) sdílené s vaším klientem pro stahování.",
  "settings.recycleBinPathExistingItems":
    "Změna složky přesune položky, které už jsou v koši, do nové složky. Položka, kterou nelze přesunout, zůstane v předchozí složce a po uložení se zobrazí zde.",
  "settings.recycleBinRetentionDays": "Uchovávat soubory v koši (dní)",
  "settings.recycleBinRetentionDaysHelp":
    "Soubory v koši jsou po tomto počtu dní trvale smazány.",
  "settings.recycleBinRetentionDaysInvalid":
    "Zadejte celé číslo dní od {{min}} do {{max}}.",
  "settings.recycleBinEffectivePaths": "Smazané soubory aktuálně míří do:",
  "settings.recycleBinNoEffectivePaths":
    "Zatím nejsou nastaveny žádné kořeny knihoven.",
  "settings.recycleBinValidationError":
    "Automatické čištění je pozastaveno, dokud se toto neopraví: {{error}}",
  "settings.recycleBinSection": "Spravovat soubory médií v koši",
  "settings.recycleBinLibraryFilter": "Knihovny",
  "settings.recycleBinEmpty": "Koš je prázdný.",
  "settings.recycleBinNoManageableLibraries":
    "Pro správu položek koše nejsou dostupné žádné knihovny.",
  "settings.recycleBinRestore": "Obnovit",
  "settings.recycleBinDelete": "Smazat",
  "settings.recycleBinEmptyAll": "Vyprázdnit koš",
  "settings.recycleBinEmptyConfirm":
    "Trvale smazat všechny položky v aktuálním zobrazení koše ({{count}})? Tuto akci nelze vrátit zpět.",
  "settings.recycleBinReason": "Důvod",
  "settings.recycleBinSize": "Velikost",
  "settings.recycleBinRecycled": "Přesunuto do koše",
  "settings.recycleBinScheduledDeletion": "Plánované smazání",
  "settings.recycleBinFilterPlaceholder":
    "Filtrovat podle názvu souboru nebo titulu",
  "settings.recycleBinFilterAria":
    "Filtrovat soubory v koši podle názvu souboru nebo titulu",
  "settings.recycleBinSelectedCount": "Vybráno: {{count}}",
  "settings.recycleBinRestoreSelected": "Obnovit vybrané",
  "settings.recycleBinDeleteSelected": "Smazat vybrané",
  "settings.recycleBinClearSelection": "Zrušit výběr",
  "settings.recycleBinNoFilterMatches":
    "Žádné soubory v koši tomuto filtru neodpovídají.",
  "settings.recycleBinSelectAllFiltered":
    "Vybrat všechny filtrované soubory v koši",
  "settings.recycleBinSelectTitle": "Vybrat {{title}}",
  "settings.recycleBinSelectFile": "Vybrat {{name}}",
  "settings.recycleBinUnassociatedFiles": "Nepřiřazené soubory",
  "settings.recycleBinFileCountOne": "{{count}} soubor",
  "settings.recycleBinFileCountOther": "Souborů: {{count}}",
  "settings.recycleBinBatchLimit":
    "Maximální počet souborů v koši vybraných najednou: {{count}}.",
  "settings.recycleBinRestoreQueuedOne":
    "Obnova {{count}} souboru v koši zařazena do fronty.",
  "settings.recycleBinRestoreQueuedOther":
    "Obnova souborů v koši zařazena do fronty ({{count}}).",
  "settings.recycleBinDeleteQueuedOne":
    "Smazání {{count}} souboru v koši zařazeno do fronty.",
  "settings.recycleBinDeleteQueuedOther":
    "Smazání souborů v koši zařazeno do fronty ({{count}}).",
  "settings.recycleBinRestoreSelectedTitle": "Obnovit soubory z koše",
  "settings.recycleBinRestoreOccupiedOne":
    "{{count}} cíl je obsazen. Zvolte, zda ponechat oba soubory, nebo před obnovením přesunout aktuální soubor do koše.",
  "settings.recycleBinRestoreOccupiedOther":
    "Obsazených cílů: {{count}}. Zvolte, zda ponechat oba soubory, nebo před obnovením přesunout aktuální soubor do koše.",
  "settings.recycleBinRestoreDescriptionOne":
    "Obnovit {{count}} vybraný soubor na jeho původní cestu.",
  "settings.recycleBinRestoreDescriptionOther":
    "Obnovit vybrané soubory ({{count}}) na jejich původní cesty.",
  "settings.recycleBinDeleteSelectedConfirmOne":
    "Trvale smazat {{count}} vybraný soubor v koši. Tuto akci nelze vrátit zpět.",
  "settings.recycleBinDeleteSelectedConfirmOther":
    "Trvale smazat vybrané soubory v koši ({{count}}). Tuto akci nelze vrátit zpět.",
  "settings.recycleBinKeepBothAndRestore": "Ponechat oba a obnovit",
  "settings.recycleBinReplaceExistingAndRestore":
    "Nahradit existující a obnovit",
  "settings.recycleBinKeepBoth": "Ponechat oba",
  "settings.recycleBinKeepBothDescription":
    "Obnovit konfliktní soubory s příponou -restored.",
  "settings.recycleBinReplaceExisting": "Nahradit existující",
  "settings.recycleBinReplaceExistingDescription":
    "Přesunout každý aktuální soubor do koše a poté na jeho místo obnovit vybraný soubor.",
  "status.recycleBinEmptied": "Koš vyprázdněn (odstraněno položek: {{count}}).",
  "status.recycleBinSettingsSaved": "Nastavení koše uloženo.",
  "settings.recycleBinRelocatedOne":
    "Přesunuta {{count}} položka z koše do nové složky.",
  "settings.recycleBinRelocatedOther":
    "Přesunuto položek z koše do nové složky: {{count}}.",
  "settings.recycleBinRelocationFailedOne":
    "{{count}} položku z koše se nepodařilo přesunout:",
  "settings.recycleBinRelocationFailedOther":
    "Nepodařilo se přesunout položky z koše ({{count}}):",
  "status.recycleBinSettingsSavedWithRelocationFailures":
    "Nastavení koše uloženo, ale některé položky z koše se nepodařilo přesunout.",
  "appUpgrade.title": "Upgrade aplikace",
  "appUpgrade.subtitle": "Správa verzí a upgradů",
  "appUpgrade.upgradeNow": "Upgradovat nyní",
  "appUpgrade.confirmTitle": "Upgradovat aplikaci?",
  "appUpgrade.confirmDescription": "Upgradovat na verzi {{version}}?",
  "appUpgrade.confirmAction": "Spustit upgrade",
  "appUpgrade.restartNotice": "Scryer se během tohoto upgradu restartuje.",
  "appUpgrade.uacNotice": "Windows zobrazí místní výzvu ke zvýšení oprávnění.",
  "appUpgrade.startFailed": "Nepodařilo se spustit upgrade.",
  "appUpgrade.targetChanged":
    "Dostupná aktualizace se změnila. Zkontrolujte ji a zkuste to znovu.",
  "appUpgrade.loading": "Načítání stavu upgradu…",
  "appUpgrade.currentVersion": "Aktuální verze",
  "appUpgrade.availableVersion": "Dostupná verze",
  "appUpgrade.noUpdate": "Není dostupná žádná aktualizace",
  "appUpgrade.installationKind": "Typ instalace",
  "appUpgrade.managementOwner": "Správce upgradu",
  "appUpgrade.owner.inApp": "Scryer",
  "appUpgrade.owner.operator": "Operátor",
  "appUpgrade.eligibility": "Způsobilost k upgradu",
  "appUpgrade.eligibility.eligible":
    "Tuto instalaci lze upgradovat přímo v aplikaci.",
  "appUpgrade.eligibility.managed_by_docker": "Tuto instalaci spravuje Docker.",
  "appUpgrade.eligibility.managed_by_homebrew":
    "Tuto instalaci spravuje Homebrew.",
  "appUpgrade.eligibility.managed_by_winget": "Tuto instalaci spravuje winget.",
  "appUpgrade.eligibility.windows_supervised":
    "Tato instalace pro Windows je spravována externě.",
  "appUpgrade.eligibility.disabled_by_operator":
    "Upgrady v aplikaci byly vypnuty operátorem.",
  "appUpgrade.eligibility.unsupported_layout":
    "Toto uspořádání instalace není podporováno.",
  "appUpgrade.eligibility.install_dir_not_writable":
    "Do instalačního adresáře nelze zapisovat.",
  "appUpgrade.eligibility.app_bundle_translocated":
    "macOS spouští Scryer z dočasné kopie. Přesuňte Scryer do složky Aplikace a otevřete ho odtud.",
  "appUpgrade.eligibility.app_bundle_read_only_volume":
    "Scryer běží na svazku pouze pro čtení. Přetáhněte Scryer do složky Aplikace a otevřete ho odtud.",
  "appUpgrade.eligibility.app_bundle_not_tray_supervised":
    "Tato aplikace nebyla spuštěna aplikací Scryer, takže ji po upgradu nelze restartovat.",
  "appUpgrade.eligibility.unknown": "Způsobilost k upgradu není k dispozici.",
  "appUpgrade.rebootRequired":
    "Pro dokončení tohoto upgradu restartujte Windows.",
  "appUpgrade.activeRun": "Upgrade probíhá",
  "appUpgrade.latestRun": "Poslední upgrade",
  "appUpgrade.runStatusLabel": "Stav: {{status}}",
  "appUpgrade.runStatus.queued": "Ve frontě",
  "appUpgrade.runStatus.discovering": "Příprava",
  "appUpgrade.runStatus.running": "Probíhá",
  "appUpgrade.runStatus.completed": "Dokončeno",
  "appUpgrade.runStatus.warning": "Dokončeno s upozorněními",
  "appUpgrade.runStatus.failed": "Selhalo",
  "appUpgrade.phase.checking": "Kontrola",
  "appUpgrade.phase.downloading": "Stahování",
  "appUpgrade.phase.verifying": "Ověřování",
  "appUpgrade.phase.staging": "Příprava",
  "appUpgrade.phase.applying": "Použití",
  "appUpgrade.phase.awaiting_elevation": "Čeká se na zvýšení oprávnění",
  "appUpgrade.phase.restarting": "Restartování",
  "appUpgrade.phase.reboot_required": "Je nutný restart",
  "appUpgrade.phase.unknown": "Pracuje se",
  "appUpgrade.downloadProgress": "Staženo {{downloaded}} z {{total}}",
  "appUpgrade.downloadedBytes": "Staženo {{downloaded}}",
  "appUpgrade.operatorGuidance": "Upgrade spravovaný operátorem",
  "appUpgrade.guidance.winget":
    "Spusťte tento příkaz v terminálu se zvýšenými oprávněními.",
  "appUpgrade.guidance.wingetCommand": "winget upgrade ScryerMedia.Scryer",
  "appUpgrade.guidance.docker":
    "Stáhněte nový image a znovu vytvořte kontejner pomocí vaší konfigurace nasazení.",
  "appUpgrade.guidance.homebrew": "Spusťte tento příkaz v terminálu.",
  "appUpgrade.guidance.homebrewCommand": "brew upgrade scryer",
  "appUpgrade.guidance.disabled":
    "Upgrady v aplikaci jsou vypnuty nastavením prostředí.",
  "appUpgrade.guidance.disabledVariable": "SCRYER_DISABLE_SELF_UPGRADE",
  "appUpgrade.guidance.other":
    "Použijte metodu upgradu poskytnutou operátorem této instalace.",
  "appUpgrade.installationKind.portable": "Přenosná",
  "appUpgrade.installationKind.directMsi": "Přímý MSI",
  "appUpgrade.installationKind.macosAppBundle": "Aplikace pro macOS",
  "appUpgrade.installationKind.docker": "Docker",
  "appUpgrade.installationKind.homebrew": "Homebrew",
  "appUpgrade.installationKind.winget": "winget",
  "appUpgrade.installationKind.windowsSupervised": "Windows (spravováno)",
  "appUpgrade.installationKind.disabled": "Vypnuto",
  "appUpgrade.installationKind.unsupported": "Nepodporováno",
  "appUpgrade.success": "Aktualizováno na verzi {{version}}",
  "appUpgrade.failed": "Aktualizace se nezdařila.",
  "settings.maintenanceRules": "Pravidla údržby",
  "nav.rules": "Pravidla",
  "settings.rulesScoring": "Pravidla hodnocení",
  "settings.maintenanceNavRules": "Pravidla",
  "settings.maintenanceNavHistory": "Historie",
  "settings.maintenanceRuleAction": "Akce",
  "settings.maintenanceRuleActionHelp":
    "Co toto pravidlo udělá, jakmile se zapnou akce údržby.",
  "settings.maintenanceRuleActionPlaceholder": "Vyberte akci",
  "settings.maintenanceRuleNoActions":
    "Nejsou k dispozici žádné akce vázané na titul.",
  "settings.maintenanceActionSequence": "Posloupnost akcí",
  "settings.maintenanceSequenceTitle": "Posloupnost akcí",
  "settings.maintenanceSequenceHelp":
    "Tyto kroky se spustí v daném pořadí poté, co subjektu uplyne ochranná doba. Prázdná posloupnost zaznamenává shody, aniž by na nich cokoli provedla.",
  "settings.maintenanceSequenceEmpty":
    "Pouze zkušební běh — nejsou nastaveny žádné kroky akce.",
  "settings.maintenanceSequenceObserveOnly": "Pouze zkušební běh",
  "settings.maintenanceSequenceSummary": "Souhrn posloupnosti",
  "settings.maintenanceSequenceSummaryTarget": "Cíle: {{target}}",
  "settings.maintenanceSequenceSummaryArming": "Po zapnutí: {{arming}}",
  "settings.maintenanceSequenceSummaryEffects": "Účinky: {{effects}}",
  "settings.maintenanceSequenceTargetTitle": "odpovídající titul",
  "settings.maintenanceSequenceTargetTitleDescendants":
    "odpovídající titul, řady a epizody",
  "settings.maintenanceSequenceTargetSeason": "odpovídající řada",
  "settings.maintenanceSequenceTargetSeasonDescendants":
    "odpovídající řada a epizody",
  "settings.maintenanceSequenceTargetEpisode": "odpovídající epizoda",
  "settings.maintenanceSequencePreviewPlan": "Seřazené účinky tohoto náhledu",
  "settings.maintenanceSequencePreviewDescendants": "včetně potomků",
  "settings.maintenanceSequencePreviewMatchedScope":
    "pouze odpovídající rozsah",
  "settings.maintenanceSequenceAdd": "Přidat krok",
  "settings.maintenanceSequencePreset": "Přednastavená posloupnost",
  "settings.maintenanceSequencePresetApply": "Nahradit přednastavením",
  "settings.maintenanceSequencePresetProfileThenSearch":
    "Změnit profil a poté vyhledat, pokud se změnil",
  "settings.maintenanceSequencePresetUnmonitorThenDelete":
    "Zrušit sledování potomků a poté smazat soubory",
  "settings.maintenanceSequenceTerminal": "Koncový",
  "settings.maintenanceSequenceDependencies": "Vyžaduje: {{steps}}",
  "settings.maintenanceSequenceIncludeDescendants": "Zahrnout řady a epizody",
  "settings.maintenanceSequenceSearchCondition": "Podmínka vyhledávání",
  "settings.maintenanceSequenceSearchUnconditional": "Vždy vyžádat vyhledávání",
  "settings.maintenanceSequenceSearchPreviousProfileChanged":
    "Pouze po změně profilu",
  "settings.maintenanceSequenceSearchRequested":
    "Tento krok vyžádá vyhledávání a zaznamená přijetí až po přijetí požadavku. Následné efekty získání a stahování lze stále pozdržet.",
  "settings.maintenanceSequenceHistoryWaiting": "čeká na dřívější krok",
  "settings.maintenanceSequenceHistorySearchRequested": "vyhledávání vyžádáno",
  "settings.maintenanceSequenceHistorySearchAccepted": "přijato",
  "settings.maintenanceSequenceHistoryJob": "úloha {{id}}",
  "settings.maintenanceSequenceCompletionAccepted": "Dokončeno po přijetí",
  "settings.maintenanceSequenceCompletionCompleted": "Dokončeno po dokončení",
  "settings.maintenanceSequenceStepUnavailable":
    "Tento uložený krok není v aktuálním katalogu k dispozici. Před uložením nebo náhledem jej odeberte, nebo obnovte kompatibilní katalog.",
  "settings.maintenanceSequenceCatalogUnavailable":
    "Katalog kroků akcí není k dispozici. Uložené kroky zůstávají viditelné, ale ukládání i náhledy neuložených kroků jsou pozastaveny, dokud se nenačte.",
  "settings.maintenanceSequenceStepUnmonitor": "Zrušit sledování",
  "settings.maintenanceSequenceStepDeleteFiles": "Smazat soubory",
  "settings.maintenanceSequenceStepChangeQualityProfile":
    "Změnit profil kvality",
  "settings.maintenanceSequenceStepSearch": "Vyhledat",
  "settings.maintenanceSequenceStepAddTags": "Přidat štítky",
  "settings.maintenanceSequenceStepRemoveTags": "Odebrat štítky",
  "settings.maintenanceSequenceStepDeleteTitleAndFiles":
    "Smazat titul a soubory",
  "settings.maintenanceRuleGraceDays": "Dny ochranné doby",
  "settings.maintenanceRuleGraceDaysHelp":
    "Počet dní od první shody každého subjektu, než se jeho akce stane splatnou.",
  "settings.maintenanceRuleTargetProfile": "Cílový profil kvality",
  "settings.maintenanceRuleTargetProfileHelp":
    "Profil, do kterého by tato akce přesunula odpovídající tituly.",
  "settings.maintenanceRuleTargetProfileIdHelp": "ID profilu kvality.",
  "settings.maintenanceRuleLibraries": "Rozsah knihoven",
  "settings.maintenanceRuleLibrariesHelp":
    "Ponecháním všech knihoven nezaškrtnutých rozšíříte rozsah pravidla na celou kolekci.",
  "settings.maintenanceRuleLibrariesAll": "Všechny knihovny",
  "settings.maintenanceRuleStorageRoot": "Kořen úložiště",
  "settings.maintenanceRuleStorageRootNone": "Není vybrán žádný kořen úložiště",
  "settings.maintenanceRuleStorageRootHelp":
    "Vyberte nakonfigurovaný kořen, ze kterého se má číst kapacita, a omezte mazání souborů jen na soubory skutečně na tomto kořeni.",
  "settings.maintenanceRuleStorageRootUnmonitorHelp":
    "Vybraný kořen omezuje pouze mazání souborů. Zrušení sledování se stále vztahuje na celý odpovídající subjekt, takže soubory na jiných kořenech zůstanou zachovány, ale přestanou být sledovány.",
  "settings.maintenanceRuleStorageRootClear": "Vymazat kořen úložiště",
  "settings.maintenanceStorageRootFileSummary":
    "Vybraný kořen · soubory: {{count}} · {{size}}",
  "settings.maintenanceRuleCreate": "Vytvořit pravidlo údržby",
  "settings.maintenanceRuleUpdate": "Aktualizovat pravidlo údržby",
  "settings.maintenanceRuleCopy": "Kopírovat jako nové pravidlo",
  "settings.maintenanceRuleCreateNew": "Vytvořit nové pravidlo údržby",
  "settings.maintenanceRuleSaveDisabledNote":
    "Nová pravidla jsou zpočátku vypnutá. Zkontrolujte náhled, vyzkoušejte zkušební běh a teprve poté pravidlo zapněte.",
  "settings.maintenanceRuleValidationRequired":
    "Je vyžadován název, zdroj porovnávání a akce.",
  "settings.maintenanceRuleConfirmDiscardTitle": "Zahodit neuložené změny?",
  "settings.maintenanceRuleConfirmDiscardDescription":
    "Editor pravidla údržby obsahuje neuložené změny, které budou ztraceny.",
  "settings.maintenanceRuleDeleting": "Smazat pravidlo údržby {{name}}?",
  "settings.noMaintenanceRulesFound": "Zatím žádná pravidla údržby.",
  "settings.maintenanceRuleRevision": "Revize",
  "settings.maintenanceActionDoNothing": "Neprovádět nic",
  "settings.maintenanceActionUnmonitorScopeKeepFiles":
    "Zrušit sledování rozsahu, soubory zachovat",
  "settings.maintenanceActionDeleteTitleAndFiles":
    "Smazat titul a jeho soubory",
  "settings.maintenanceActionUnmonitorTitleDeleteAllFiles":
    "Zrušit sledování titulu, smazat všechny soubory",
  "settings.maintenanceActionUnmonitorShowDeleteExistingFiles":
    "Zrušit sledování seriálu, smazat existující soubory",
  "settings.maintenanceActionUnmonitorScopeDeleteFiles":
    "Zrušit sledování rozsahu, smazat jeho soubory",
  "settings.maintenanceActionUnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty":
    "Zrušit sledování řady, smazat její soubory a poté smazat seriál, je-li prázdný",
  "settings.maintenanceActionUnmonitorSeasonThenUnmonitorShowIfEmpty":
    "Zrušit sledování řady a poté zrušit sledování seriálu, je-li prázdný",
  "settings.maintenanceActionChangeQualityProfileAndSearchIfChanged":
    "Změnit profil kvality a vyhledat, pokud se změnil",
  "settings.maintenancePreviewTitle": "Náhled",
  "settings.maintenancePreviewSubtitle":
    "Spustí porovnávání vůči titulům, řadám nebo epizodám v jeho rozsahu. Náhled ukazuje, co by odpovídalo, a nic nemění.",
  "settings.maintenancePreviewSource": "Porovnávání",
  "settings.maintenancePreviewSourceStored": "Uložené pravidlo",
  "settings.maintenancePreviewSourceDraft": "Koncept z editoru",
  "settings.maintenancePreviewLibrary": "Knihovna",
  "settings.maintenancePreviewLibraryPlaceholder": "Vyberte knihovnu",
  "settings.maintenancePreviewLimit": "Subjekty",
  "settings.maintenancePreviewLimitHelp": "Subjektů na jeden běh: až {{max}}.",
  "settings.maintenancePreviewRun": "Spustit náhled",
  "settings.maintenancePreviewRunning": "Probíhá náhled...",
  "settings.maintenancePreviewEmpty": "Vyberte knihovnu a spusťte náhled.",
  "settings.maintenancePreviewNoTitles": "Nebyl vyhodnocen žádný titul.",
  "settings.maintenancePreviewColOutcome": "Výsledek",
  "settings.maintenancePreviewColReasons": "Kódy důvodu",
  "settings.maintenancePreviewOutcomeMatch": "SHODA",
  "settings.maintenancePreviewOutcomeNoMatch": "BEZ SHODY",
  "settings.maintenancePreviewOutcomeUnknown": "NEZNÁMÉ",
  "settings.maintenancePreviewOutcomeError": "CHYBA",
  "settings.maintenancePreviewMeta":
    "Vyhodnoceno {{time}} · porovnávání {{hash}}",
  "settings.maintenancePreviewRuleSelect": "Pravidlo",
  "settings.maintenancePreviewRuleSelectPlaceholder":
    "Vyberte uložené pravidlo",
  "settings.maintenanceRuleUpdated": "Pravidlo údržby aktualizováno.",
  "settings.maintenanceRuleCreated":
    "Pravidlo údržby vytvořeno a uloženo jako vypnuté.",
  "settings.maintenanceRuleDeleted": "Pravidlo údržby {{name}} smazáno.",
  "settings.maintenanceArmingNone": "Žádné účinky",
  "settings.maintenanceArmingReversible": "Aktualizace knihovny",
  "settings.maintenanceArmingDestructive": "Mazání souborů",
  "settings.maintenanceRuleState": "Stav",
  "settings.maintenanceRuleStateHelp":
    "Vypnutá pravidla se nikdy nevyhodnocují. Zkušební běh zaznamenává kandidáty, aniž by na nich cokoli prováděl. Zapnutá pravidla jednají na svých kandidátech po uplynutí každé ochranné doby.",
  "settings.maintenanceRuleStateUpdated": "Aktualizováno: {{name}}.",
  "settings.maintenanceRuleStateDisabled": "Vypnuto",
  "settings.maintenanceRuleStateDisabledHelp": "Nikdy nevyhodnocováno.",
  "settings.maintenanceRuleStateDryRun": "Zkušební běh",
  "settings.maintenanceRuleStateDryRunHelp":
    "Zaznamenává kandidáty, neprovádí žádnou akci.",
  "settings.maintenanceRuleStateEnabled": "Zapnuto",
  "settings.maintenanceRuleStateEnabledHelp":
    "Jedná na kandidátech po uplynutí jejich ochranné doby.",
  "settings.maintenanceRuleEditedSinceEnabled":
    "Upraveno od zapnutí. Zkontrolujte kandidáty a zapněte jej znovu.",
  "settings.maintenanceRuleSystemOwned": "Vestavěné",
  "settings.expiredRequestLeasesHelp":
    "Smaže požadovaný titul a jeho soubory poté, co vyprší nárok žádosti a uplyne ochranná doba. Změna ochranné doby nastaví pravidlo na zkušební běh, dokud jej znovu nezapnete. Stejně jako každé pravidlo údržby funguje jen tehdy, když je údržba zapnutá.",
  "settings.maintenanceSwitchEnabled": "Údržba",
  "settings.maintenanceSwitchEnabledHelp":
    "Vyhodnocuje pravidla a zobrazuje jejich kandidáty na této instanci.",
  "settings.maintenanceSwitchActionsPaused": "Pozastavit všechny akce",
  "settings.maintenanceSwitchActionsPausedHelp":
    "Během pozastavení zapnutá pravidla nadále zaznamenávají kandidáty, ale nic se neruší ze sledování, nemění ani nemaže.",
  "settings.maintenanceSwitchesLocked":
    "Čtení přepínačů údržby vyžaduje oprávnění k systémovému nastavení. Zeptejte se správce, zda je údržba zapnutá a zda jsou akce pozastavené.",
  "settings.maintenanceSwitchesUpdated": "Nastavení údržby aktualizováno.",
  "settings.maintenanceUnpauseConfirmTitle": "Obnovit akce údržby?",
  "settings.maintenanceUnpauseConfirmBody":
    "Zapnutá pravidla budou jednat na svých kandidátech po uplynutí každé ochranné doby, včetně pravidel, která mažou soubory.",
  "settings.maintenanceUnpauseConfirmAction": "Obnovit akce",
  "settings.maintenanceCandidatesEmptyMaintenanceOff":
    "Pro tuto instanci není zapnutá údržba, takže se nezobrazují žádní kandidáti.",
  "settings.maintenanceCandidatesEmptyAllDisabled":
    "Všechna pravidla jsou vypnutá, takže se nic nevyhodnocuje. Nejprve nastavte některé pravidlo na zkušební běh.",
  "settings.maintenanceArmDestructiveIntro":
    "{{name}} maže soubory. Po zapnutí bude jednat na níže uvedených kandidátech po uplynutí každé ochranné doby, pokud nejsou akce pozastaveny.",
  "settings.maintenanceArmDestructiveTitle":
    "Zapnout pravidlo, které maže soubory?",
  "settings.maintenanceArmDestructiveLoading":
    "Počítají se kandidáti, které toto pravidlo již drží…",
  "settings.maintenanceArmDestructiveCount":
    "Kandidátů, na kterých lze jednat: {{count}}.",
  "settings.maintenanceArmDestructiveNoCandidates":
    "Toto pravidlo aktuálně nedrží žádné kandidáty, na kterých by bylo možné jednat.",
  "settings.maintenanceArmDestructiveCountChanged":
    "Počet kandidátů se změnil: toto pravidlo nyní drží {{count}}. Pro pokračování potvrďte aktuální počet.",
  "settings.maintenanceArmDestructiveAcknowledge":
    "Zapnout mazání souborů pro tituly: {{count}}.",
  "settings.maintenanceArmDestructiveConfirm": "Zapnout",
  "settings.maintenanceCandidatesTitle": "Brzy odejde",
  "settings.maintenanceCandidatesSubtitle":
    "Tituly, kterým odpovídá nějaké pravidlo, nejdříve splatné jako první. Zatím se s nimi nic neprovedlo.",
  "settings.maintenanceCandidatesFilterRule": "Pravidlo",
  "settings.maintenanceCandidatesFilterState": "Stav",
  "settings.maintenanceCandidatesFilterLibrary": "Knihovna",
  "settings.maintenanceCandidatesAllRules": "Všechna pravidla",
  "settings.maintenanceCandidatesAllStates": "Všechny stavy",
  "settings.maintenanceCandidatesAllLibraries": "Všechny knihovny",
  "settings.maintenanceCandidatesColRule": "Pravidlo",
  "settings.maintenanceCandidatesColState": "Stav",
  "settings.maintenanceCandidatesColFirstMatched": "První shoda",
  "settings.maintenanceCandidatesColDue": "Splatnost",
  "settings.maintenanceCandidatesHeldSince": "Drženo od {{time}}",
  "settings.maintenanceCandidatesEmpty":
    "Žádní kandidáti neodpovídají těmto filtrům.",
  "settings.maintenanceCandidateExclude": "Vyloučit tento titul",
  "settings.maintenanceCandidateStateObserving": "Sledováno",
  "settings.maintenanceCandidateStatePendingAction": "Akce čeká na vyřízení",
  "settings.maintenanceCandidateStateDue": "Splatné",
  "settings.maintenanceCandidateStateExecuting": "Probíhá",
  "settings.maintenanceCandidateStateSucceeded": "Úspěšné",
  "settings.maintenanceCandidateStateFailed": "Neúspěšné",
  "settings.maintenanceCandidateStateCanceled": "Zrušeno",
  "settings.maintenanceCandidateStateExcluded": "Vyloučeno",
  "settings.maintenanceCandidateStateBlocked": "Blokováno",
  "settings.maintenanceCountdownDueNow": "Splatné nyní",
  "settings.maintenanceCountdownInMinutes": "za {{count}} min",
  "settings.maintenanceCountdownInHours": "za {{count}} h",
  "settings.maintenanceCountdownInDays": "za {{count}} d",
  "settings.maintenanceCountdownOverdueMinutes": "{{count}} min po splatnosti",
  "settings.maintenanceCountdownOverdueHours": "{{count}} h po splatnosti",
  "settings.maintenanceCountdownOverdueDays": "{{count}} d po splatnosti",
  "settings.maintenanceRunsTitle": "Běhy a historie",
  "settings.maintenanceRunsSubtitle":
    "Nedávné vyhodnocovací běhy a pokusy o akce, nejnovější první.",
  "settings.maintenanceRunScope": "Rozsah",
  "settings.maintenanceRunScopeAll": "Každé pravidlo",
  "settings.maintenanceRunEvaluationNow": "Vyhodnotit nyní",
  "settings.maintenanceRunEvaluationRunning": "Probíhá vyhodnocování…",
  "settings.maintenanceRunActionsNow": "Spustit akce nyní",
  "settings.maintenanceRunActionsRunning": "Probíhají akce…",
  "settings.maintenanceRunStarted": "Běh spuštěn.",
  "settings.maintenanceRunNotStarted": "Nic se nespustilo: {{message}}",
  "settings.maintenanceEvaluationRunsTitle": "Vyhodnocovací běhy",
  "settings.maintenanceEvaluationRunsEmpty":
    "Zatím nejsou zaznamenány žádné vyhodnocovací běhy.",
  "settings.maintenanceActionRunsTitle": "Běhy akcí",
  "settings.maintenanceActionRunsEmpty":
    "Zatím nejsou zaznamenány žádné běhy akcí.",
  "settings.maintenanceRunColStarted": "Spuštěno",
  "settings.maintenanceRunColStatus": "Stav",
  "settings.maintenanceRunColCounts": "Počty",
  "settings.maintenanceRunColDuration": "Doba trvání",
  "settings.maintenanceRunColError": "Chyba",
  "settings.maintenanceRunCounts":
    "Vyhodnoceno {{evaluated}} · shoda {{matched}} · bez shody {{noMatch}} · neznámé {{unknown}} · chyb {{errors}}",
  "settings.maintenanceRunDuration": "{{seconds}} s",
  "settings.maintenanceRunRevision": "rev {{revision}}",
  "settings.maintenanceRunStatusRunning": "Probíhá",
  "settings.maintenanceRunStatusSucceeded": "Úspěšné",
  "settings.maintenanceRunStatusFailed": "Neúspěšné",
  "settings.maintenanceRunStatusHeld": "Pozdrženo",
  "settings.maintenanceRunStatusAlreadySatisfied": "Již splněno",
  "settings.maintenanceRunStatusSkipped": "Přeskočeno",
  "settings.maintenanceActionRunColAttempt": "Pokus",
  "settings.maintenanceActionRunColHoldReason": "Důvod pozdržení",
  "settings.maintenanceExclusionsTitle": "Výjimky",
  "settings.maintenanceExclusionsSubtitle":
    "Tituly, na které pravidlo údržby nesmí nikdy jednat. Globální výjimka platí pro každé pravidlo.",
  "settings.maintenanceExclusionsEmpty": "Žádné výjimky.",
  "settings.maintenanceExclusionColScope": "Rozsah",
  "settings.maintenanceExclusionColReason": "Důvod",
  "settings.maintenanceExclusionColCreated": "Přidáno",
  "settings.maintenanceExclusionScopeGlobal": "Každé pravidlo",
  "settings.maintenanceExclusionRemove": "Odebrat výjimku",
  "settings.maintenanceExclusionRemoved": "Výjimka pro {{title}} odebrána.",
  "settings.maintenanceExclusionRemoving": "Odebrat výjimku pro {{title}}?",
  "settings.maintenanceExcludeTitle": "Vyloučit z údržby",
  "settings.maintenanceExcludeBody":
    "{{title}} bude od příštího vyhodnocení vyloučen a jeho kandidát se přesune do stavu vyloučeno.",
  "settings.maintenanceExcludeScope": "Rozsah",
  "settings.maintenanceExcludeScopeGlobal": "Každé pravidlo",
  "settings.maintenanceExcludeReason": "Důvod",
  "settings.maintenanceExcludeReasonPlaceholder":
    "Proč je tento titul vyloučen (volitelné)",
  "settings.maintenanceExcludeConfirm": "Vyloučit",
  "settings.maintenanceExcluded": "Vyloučeno z údržby: {{title}}.",
  "settings.refMaintTitle": "Referenční vstupy údržby",
  "settings.refMaintSubtitle":
    "Kliknutím rozbalte každý fakt, který může porovnávání údržby číst.",
  "settings.refMaintIntro":
    "Každé pravidlo údržby dostane jeden vstupní objekt na subjekt: input.subject určuje, co se vyhodnocuje, input.library udává, kde to žije, input.facts nese snímek faktů a obálka vyhodnocení sedí přímo na input. Fakt je obyčejná hodnota a klíč jednoduše chybí, když není co hlásit — pište tedy input.facts.monitored, ne kontrolu stavu. Pokud pravidlo čte fakt, který Scryer pro daný subjekt nedokázal zjistit, Scryer za vás subjekt pozdrží; tuto pojistku nikdy nemusíte psát sami.",
  "settings.refMaintOutputTitle": "Výstup pravidla",
  "settings.refMaintOutputIntro":
    "Porovnávání definuje až tři názvy. Cokoli dalšího v balíčku je ignorováno a rozhodnutí, které nemá přesně tento tvar, je odmítnuto, ne převedeno.",
  "settings.refMaintOutputMatch":
    "match — boolean. Pokud jej ponecháte nedefinovaný, znamená to, že subjekt neodpovídal.",
  "settings.refMaintOutputUnknown":
    "unknown — boolean. Nastavte jej, když samotné pravidlo nevidí dost na rozhodnutí; unknown má vždy přednost před shodou. Fakty, které Scryer nedokázal zjistit, jsou za vás už vyřešeny, takže tohle slouží jen pro vaše vlastní úsudky.",
  "settings.refMaintOutputReasons":
    "reasons — sada krátkých strojových kódů vysvětlujících rozhodnutí. Ukládají se ke kandidátovi a zobrazují se v náhledu.",
  "settings.refMaintOutputNoPackage":
    "Nepište řádek package ani import. Scryer oba přidává při ukládání pravidla a pro editor je zase odstraní.",
  "settings.refMaintSectionInput": "— Obálka vyhodnocení",
  "settings.refMaintInputSchemaVersion":
    "Verze vstupního dokumentu údržby, pro kterou je toto pravidlo napsáno",
  "settings.refMaintInputEvaluationTime":
    "Okamžik ve formátu RFC3339 zachycený jednou za běh, takže každý subjekt porovnává vůči stejným hodinám. Použijte jej pro výpočty s daty — time.now_ns() existuje, ale stejný subjekt by se při opakování mohl rozhodnout jinak.",
  "settings.refMaintSectionSubject":
    "— Titul, řada nebo epizoda ve vyhodnocení",
  "settings.maintenanceFileSummary": "Soubory: {{count}} · {{size}}",
  "settings.maintenanceTestTitle": "Vyberte titul k otestování",
  "settings.maintenanceNoSubjects":
    "Pro vybraný rozsah nejsou v tomto titulu žádné subjekty.",
  "settings.maintenancePreviewExcluded": "Vyloučeno z akcí",
  "settings.maintenancePreviewEstimatedDue": "Při shodě nyní: {{time}}",
  "settings.maintenancePreviewDue": "Konec ochranné doby: {{time}}",
  "settings.maintenancePreviewSubject": "Testovaný subjekt",
  "settings.maintenancePreviewSample": "Vzorek knihovny",
  "settings.maintenanceRuleScope": "Rozsah pravidla",
  "settings.maintenanceRuleTemplate": "Šablona",
  "settings.maintenanceRuleTemplatePlaceholder": "Začít od šablony",
  "settings.maintenanceScopeSeason": "Řada",
  "settings.maintenanceScopeEpisode": "Epizoda",
  "settings.maintenanceFileDeleted": "Smazáno",
  "settings.maintenanceFilePending": "Čeká",
  "settings.maintenanceFileResults": "Výsledky souborů",
  "settings.maintenanceFileProgress":
    "Smazáno: {{completed}} · zbývá: {{remaining}}",
  "settings.refMaintSubjectId":
    "ID titulu, kolekce nebo epizody Scryer pro vybraný rozsah",
  "settings.refMaintCollectionId":
    "ID vlastnící kolekce řady pro subjekty typu řada a epizoda, je-li známo",
  "settings.refMaintSubjectKind":
    "Granularita vyhodnocení: titul, řada nebo epizoda",
  "settings.refMaintSubjectTitleId":
    "Identifikátor Scryer titulu, ke kterému subjekt patří",
  "settings.refMaintSubjectSeasonNumber":
    "Číslo řady pro subjekty typu řada a epizoda, je-li známo",
  "settings.refMaintSubjectEpisodeId":
    "Identifikátor epizody; přítomen jen u subjektů typu epizoda",
  "settings.refMaintSubjectFacet":
    "Mediální oblast titulu (film, seriál, anime)",
  "settings.refMaintSubjectName": "Zobrazovaný název titulu, řady nebo epizody",
  "settings.refMaintSubjectYear": "Rok vydání titulu, je-li známý",
  "settings.refMaintSectionLibrary": "— Knihovna, ve které subjekt žije",
  "settings.refMaintLibraryId": "Identifikátor knihovny obsahující subjekt",
  "settings.refMaintLibraryName": "Zobrazovaný název této knihovny",
  "settings.refMaintSectionFacts":
    "— Snímek faktů; každý fakt je obyčejná hodnota a klíč chybí, pokud fakt není k dispozici nebo jej nelze zjistit",
  "settings.refMaintFactsMonitored": "Zda je subjekt sledován",
  "settings.refMaintFactsTags":
    "Uživatelsky definované štítky zděděné z vlastnícího titulu, malými písmeny, z registru štítků, který spravuje administrátor v Nastavení; vyhrazené položky scryer: jsou vyloučeny",
  "settings.refMaintFactsQualityProfileId":
    "Profil kvality zděděný z vlastnícího titulu",
  "settings.refMaintFactsAddedAt": "Kdy byl subjekt přidán do knihovny",
  "settings.refMaintFactsAddedByUserId":
    "Který uživatel subjekt přidal; chybí, pokud jej vytvořilo prohledávání nebo objevení",
  "settings.refMaintFactsAddedByUsername":
    "Jméno uživatele, který subjekt přidal",
  "settings.refMaintFactsRequested":
    "Zda subjekt vznikl na základě mediální žádosti",
  "settings.refMaintFactsRequestedByUserIds":
    "Uživatelé, kteří subjekt požadovali: zadavatel plus případní další žadatelé",
  "settings.refMaintFactsRequestedByUsernames": "Jména těchto žadatelů",
  "settings.refMaintFactsFirstImportedAt":
    "Kdy byl pro subjekt poprvé importován soubor",
  "settings.refMaintFactsLastUpgradedAt":
    "Kdy subjekt naposledy obdržel upgrade",
  "settings.refMaintFactsHasFile":
    "Zda má subjekt na disku alespoň jeden soubor",
  "settings.refMaintFactsFileCount": "Kolik souborů má subjekt na disku",
  "settings.refMaintFactsTotalFileSizeBytes":
    "Celková velikost souborů subjektu na disku",
  "settings.refMaintFactsFiles":
    "Soubory subjektu, s detailem média pro každý soubor",
  "settings.refMaintFactsEpisodeCount": "Kolik epizod subjekt pokrývá",
  "settings.refMaintFactsEpisodeFileCount": "Kolik z těchto epizod má soubor",
  "settings.refMaintFactsMonitoredEpisodeCount":
    "Kolik z těchto epizod je sledováno",
  "settings.refMaintFactsEpisodePositionByAirDate":
    "Pořadí epizody podle data vysílání v rámci seriálu",
  "settings.refMaintFactsSeasonPositionByAirDate":
    "Pořadí řady podle nejnovější způsobilé stažené epizody",
  "settings.refMaintFactsStorageRootId":
    "Nakonfigurovaný kořen vybraný pro kapacitu úložiště a mazání souborů",
  "settings.refMaintFactsStorageAvailableBytes":
    "Bajty aktuálně dostupné na vybraném kořeni",
  "settings.refMaintFactsStorageTotalBytes":
    "Celková kapacita vybraného kořene v bajtech",
  "settings.refMaintFactsStorageAvailablePercent":
    "Procento aktuálně dostupné kapacity vybraného kořene",
  "settings.refMaintFactsActiveDownloads":
    "Zda má subjekt probíhající stahování",
  "settings.refMaintFactsWatchedByUserIds":
    "Uživatelé, kteří tuto epizodu shlédli; u řad musí stejní uživatelé shlédnout každou odvysílanou epizodu, včetně chybějících souborů. Budoucí epizody jsou vyloučeny; nevyřešená data vysílání nebo neúplné signály jsou neznámé. Prázdné řady nejsou nikdy dokončené.",
  "settings.refMaintFactsLastWatchedAt":
    "Kdy byl subjekt naposledy přehrán kýmkoli",
  "settings.refMaintFactsWatchedByAnyRequester":
    "Zda jej přehrál alespoň jeden ze žadatelů o subjekt",
  "settings.refMaintFactsWatchedByAllRequesters":
    "Zda jej přehráli úplně všichni žadatelé o subjekt",
  "settings.refMaintFactsRequestLeaseState":
    "Souhrnný stav nároků žádostí subjektu: žádný, spící, aktivní nebo vypršelý",
  "settings.refMaintFactsRequestLeaseExpiresAt":
    "Kdy vyprší poslední živý nárok na subjekt",
  "settings.refMaintFactsActiveRetentionClaims":
    "Kolik nároků žádostí stále drží subjekt",
  "settings.refMaintFactsListsAddedByList":
    "Zda subjekt do knihovny přidal seznam, a ne osoba",
  "settings.refMaintFactsListsOnEnabledList":
    "Zda subjekt stále uvádí nějaký zapnutý seznam, veřejný nebo osobní",
  "settings.refMaintFactsListsNames":
    "Názvy zapnutých veřejných seznamů, které subjekt stále uvádějí",
  "settings.refMaintFactsListsLeftAll":
    "Zda subjekt opustily úplně všechny seznamy, které jej kdy uváděly",
  "settings.refMaintFactsListsLeftAt": "Kdy subjekt opustil poslední seznam",
  "settings.refMaintFactsListsLastListName":
    "Veřejný seznam, který subjekt naposledy opustil",
  "settings.refMaintFactsListsEnabledIds":
    "Identifikátory zapnutých seznamů, veřejných nebo osobních, které subjekt stále uvádějí",
  "settings.refMaintFactsListsPublicCount":
    "Kolik veřejných seznamů subjekt stále uvádí",
  "settings.refMaintFactsListsPersonalCount":
    "Kolik osobních seznamů subjekt stále uvádí",
  "settings.refMaintFactsListsFirstSeenAt":
    "Kdy subjekt poprvé uvedl seznam, který jej stále drží",
  "settings.refMaintFactsListsLastSeenAt":
    "Kdy subjekt naposledy uvedl seznam, který jej stále drží",
  "settings.refMaintFactsListsDaysOnList":
    "Celé dny od chvíle, kdy subjekt poprvé uvedl seznam, který jej stále drží",
  "settings.refMaintFactsListsBestRank":
    "Nejlepší (nejnižší) pořadí, které subjektu dává kterýkoli seznam, jenž jej stále drží",
  "settings.refMaintFactsListsOnLeave":
    "Co zapnuté seznamy držící subjekt udělají, když jej opustí",
  "settings.refMaintFactsListsLeftHandled":
    "Zda byla u každého odchodu ze seznamu uplatněna akce při odchodu",
  "settings.refMaintFactsListsMembershipStates":
    "Stavy subjektu na seznamech, které jej stále drží",
  "settings.refMaintFactsRequestOrigin":
    "Odkud pocházela žádost, která titul vytvořila",
  "settings.refMaintFactsRequestListName":
    "Veřejný seznam, který titul požadoval",
  "settings.refMaintObsStatus":
    "known, absent nebo unknown. absent znamená, že zdroj odpověděl a hodnota neexistuje; unknown znamená, že to Scryer nedokázal zjistit. Otestujte toto před čtením value — neznámý fakt se nikdy nepřevádí na false, 0 ani prázdný řetězec.",
  "settings.refMaintSectionObservations":
    "— Stejné fakty jako plné obálky, pro pravidla, která potřebují rozlišit absent od unknown nebo přečíst důvod. Čtení faktu zde vyřadí daný fakt z automatického pozdržení: přebíráte odpovědnost za trojhodnotovou odpověď.",
  "settings.refMaintSectionObsFiles":
    "— Jedna položka na soubor v input.observations.files.value",
  "settings.refMaintObsObservedAt":
    "Kdy byla hodnota zjištěna, pokud zdroj zaznamenal čas",
  "settings.refMaintObsReason":
    "Stabilní strojový kód vysvětlující, proč je fakt neznámý nebo proč chybějící fakt nemá hodnotu",
  "settings.refMaintFactsMonitoredValue": "True, pokud je subjekt sledován",
  "settings.refMaintFactsTagsValue":
    "Uživatelsky definované názvy štítků malými písmeny z administrátorského registru štítků; nikdy vyhrazené položky scryer:",
  "settings.refMaintFactsQualityProfileIdValue":
    "Identifikátor přiřazeného profilu kvality",
  "settings.refMaintFactsAddedAtValue":
    "Časové razítko RFC3339, kdy byl subjekt přidán",
  "settings.refMaintFactsAddedByUserIdValue":
    "Identifikátor uživatele, který subjekt přidal; důvod je title_added_by_system, pokud to neudělal nikdo",
  "settings.refMaintFactsAddedByUsernameValue":
    "Uživatelské jméno tohoto uživatele; unknown s důvodem user_not_found, pokud účet zanikl",
  "settings.refMaintFactsRequestedValue":
    "True, pokud subjekt vytvořila alespoň jedna mediální žádost; jinak potvrzené false",
  "settings.refMaintFactsRequestedByUserIdsValue":
    "Identifikátory žadatelů, bez duplicit; prázdný seznam, pokud subjekt nebyl požadován",
  "settings.refMaintFactsRequestedByUsernamesValue":
    "Uživatelská jména žadatelů; unknown s důvodem user_not_found, pokud některý z těchto účtů zanikl",
  "settings.refMaintFactsFirstImportedAtValue":
    "Časové razítko RFC3339 prvního importu",
  "settings.refMaintFactsLastUpgradedAtValue":
    "Časové razítko RFC3339 posledního upgradu",
  "settings.refMaintFactsHasFileValue":
    "True, pokud je na disku alespoň jeden soubor",
  "settings.refMaintFactsFileCountValue": "Počet souborů na disku",
  "settings.refMaintFactsTotalFileSizeBytesValue":
    "Celková velikost těchto souborů v bajtech",
  "settings.refMaintFactsFilesValue":
    "Jedna položka na soubor; viz input.observations.files.value[] níže",
  "settings.refMaintFactsEpisodeCountValue": "Počet epizod v rozsahu",
  "settings.refMaintFactsEpisodeFileCountValue":
    "Počet z těchto epizod, které mají soubor",
  "settings.refMaintFactsMonitoredEpisodeCountValue":
    "Počet z těchto epizod, které jsou sledovány",
  "settings.refMaintFactsEpisodePositionByAirDateValue":
    "Pořadí 1 má nejnovější způsobilá stažená, odvysílaná nespeciální epizoda. Při shodě dat vysílání rozhoduje číslo řady a epizody, poté stabilní ID.",
  "settings.refMaintFactsSeasonPositionByAirDateValue":
    "Pořadí 1 má řada s nejnovější způsobilou staženou, odvysílanou nespeciální epizodou. Chráněné soubory stále zabírají pořadí.",
  "settings.refMaintFactsStorageRootIdValue":
    "Trvalý identifikátor nakonfigurovaného kořene knihovny vybraný v této revizi",
  "settings.refMaintFactsStorageAvailableBytesValue":
    "Dostupná kapacita v bajtech, zjištěná přímo z vybraného kořene",
  "settings.refMaintFactsStorageTotalBytesValue":
    "Celková kapacita v bajtech, zjištěná přímo z vybraného kořene",
  "settings.refMaintFactsStorageAvailablePercentValue":
    "Dostupná kapacita dělená celkovou kapacitou, od 0 do 100",
  "settings.refMaintFactsActiveDownloadsValue":
    "True, pokud u subjektu právě probíhá stahování",
  "settings.refMaintFactsWatchedByUserIdsValue":
    "Identifikátory všech, kdo to přehráli, seřazené a bez duplicit; prázdný seznam znamená, že to nikdo neudělal. Dokončení řady vyžaduje, aby stejná osoba shlédla každou odvysílanou epizodu, včetně chybějících souborů. Budoucí epizody jsou vyloučeny; chybějící historie sledování nebo nevyřešená data vysílání jsou neznámé. Fakty sledování jsou neznámé u titulů v rozsahu seriálu (show_watch_rollup_unavailable), když není připojen žádný mediální server (no_media_server_connection), když se připojení nikdy čistě nesynchronizovalo (signal_sync_never_succeeded), když je poslední čistá synchronizace starší než 48 hodin (signals_stale), když nikdo nemá ověřený propojený účet mediálního serveru (no_linked_participants) a když přehrání nelze přiřadit uživateli Scryer (signal_identity_missing).",
  "settings.refMaintFactsLastWatchedAtValue":
    "Časové razítko RFC3339 posledního přehrání; chybí s důvodem never_watched, pokud to nikdo nepřehrál",
  "settings.refMaintFactsWatchedByAnyRequesterValue":
    "True, pokud to přehrál alespoň jeden žadatel; chybí s důvodem title_not_requested, pokud subjekt nikdo nepožadoval, a unknown s důvodem requester_not_linked, pokud žadatel nemá propojený účet mediálního serveru",
  "settings.refMaintFactsWatchedByAllRequestersValue":
    "True, pokud to přehráli úplně všichni žadatelé; chybí s důvodem title_not_requested, pokud subjekt nikdo nepožadoval, a unknown s důvodem requester_not_linked, pokud žadatel nemá propojený účet mediálního serveru",
  "settings.refMaintFactsRequestLeaseStateValue":
    "none, pokud pro subjekt nikdy nebyl zapsán žádný nárok žádosti, dormant, pokud nárok čeká na první import, active, dokud nárok běží, a expired, jakmile vyprší úplně každý nárok. Nárok, který byl uvolněn nebo převeden, je stažen, ne vyčerpán, takže stav expired nezpůsobí. Vypršení se měří vůči času vyhodnocení, takže nárok se jeví jako expired ve chvíli, kdy se jeho okno uzavře.",
  "settings.refMaintFactsRequestLeaseExpiresAtValue":
    "Časové razítko RFC3339 nejpozdějšího vypršení mezi živými nároky; chybí s důvodem no_live_lease, pokud subjekt nic nedrží",
  "settings.refMaintFactsActiveRetentionClaimsValue":
    "Počet nároků, které stále drží subjekt, včetně spících",
  "settings.refMaintFactsListsAddedByListValue":
    "True, pokud titul vytvořila synchronizace seznamu; unknown s důvodem list_store_unreadable, pokud nelze přečíst úložiště seznamů",
  "settings.refMaintFactsListsOnEnabledListValue":
    "True, dokud subjekt drží alespoň jeden zapnutý seznam",
  "settings.refMaintFactsListsNamesValue":
    "Pouze názvy veřejných seznamů, seřazené; osobní seznam člena se nikdy nejmenuje",
  "settings.refMaintFactsListsLeftAllValue":
    "True, jakmile subjekt opustily úplně všechny seznamy, které jej uváděly; false, dokud jej drží alespoň jeden nebo pokud žádný seznam nikdy neuváděl",
  "settings.refMaintFactsListsLeftAtValue":
    "Časové razítko RFC3339; chybí s důvodem not_left_every_list, dokud subjekt drží jakýkoli seznam",
  "settings.refMaintFactsListsLastListNameValue":
    "Název tohoto veřejného seznamu; chybí s důvodem no_public_list_left, pokud subjekt nikdy neopustil veřejný seznam",
  "settings.refMaintFactsListsEnabledIdsValue":
    "Identifikátory seznamů, seřazené; nikdy název seznamu",
  "settings.refMaintFactsListsPublicCountValue":
    "Počet veřejných seznamů, zapnutých nebo pozastavených",
  "settings.refMaintFactsListsPersonalCountValue":
    "Počet osobních seznamů, zapnutých nebo pozastavených",
  "settings.refMaintFactsListsFirstSeenAtValue":
    "Časové razítko RFC3339; chybí s důvodem not_on_a_list, pokud subjekt nedrží žádný seznam",
  "settings.refMaintFactsListsLastSeenAtValue":
    "Časové razítko RFC3339; chybí s důvodem not_on_a_list, pokud subjekt nedrží žádný seznam",
  "settings.refMaintFactsListsDaysOnListValue":
    "Celé dny do času vyhodnocení; chybí s důvodem not_on_a_list, pokud subjekt nedrží žádný seznam",
  "settings.refMaintFactsListsBestRankValue":
    "Číslo pořadí; chybí s důvodem no_list_rank, pokud jej žádný seznam, který subjekt drží, neřadí",
  "settings.refMaintFactsListsOnLeaveValue":
    "Seřazené hodnoty z keep, log, unmonitor a tag",
  "settings.refMaintFactsListsLeftHandledValue":
    "True, jakmile byl vyřízen každý odchod; chybí s důvodem no_list_left, pokud subjekt neopustil žádný seznam",
  "settings.refMaintFactsListsMembershipStatesValue":
    "Seřazené stavy, jako in_library, added, requested nebo held",
  "settings.refMaintFactsRequestOriginValue":
    "Jedno z manual, public_list nebo personal_list; chybí s důvodem title_not_requested, pokud titul nevytvořila žádná žádost",
  "settings.refMaintFactsRequestListNameValue":
    "Název tohoto veřejného seznamu; chybí s důvodem no_public_request_list u ručních a osobních žádostí ze seznamu, protože osobní seznam se nikdy nejmenuje",
  "settings.refMaintSectionFiles":
    "— Jedna položka na soubor v input.facts.files",
  "settings.refMaintFileSizeBytes": "Velikost souboru v bajtech",
  "settings.refMaintFileQuality":
    "Zjištěná kvalita souboru (například 2160P, 1080P)",
  "settings.refMaintFileVideoCodec":
    "Video kodek souboru (například H.265, H.264)",
  "settings.refMaintFileIs3d":
    "True, když mediální metadata deklarují video 3D. False znamená nezjištěno, včetně souborů dosud neanalyzovaných.",
  "settings.refMaintFileVideoWidth": "Šířka videa v pixelech",
  "settings.refMaintFileVideoHeight": "Výška videa v pixelech",
  "settings.refMaintFileAudioLanguages": "Jazyky zvuku přítomné v souboru",
  "settings.refMaintFileSubtitleLanguages": "Jazyky titulků přítomné v souboru",
  "settings.refMaintFileAddedAt":
    "Časové razítko RFC3339, kdy byl soubor přidán",
  "settings.refMaintFactsSeriesMovies":
    "Filmy propojené do časové osy tohoto seriálu, každý s vlastními štítky. Přítomno pouze u seriálů: u subjektu film klíč zcela chybí, u seriálu bez propojených filmů je prázdné, ale známé. add_tags a remove_tags tyto nikdy nezapisují: působí na titul subjektu.",
  "settings.refMaintFactsSeriesMoviesValue":
    "Jedna položka na propojený film; viz input.observations.series_movies.value[] níže",
  "settings.refMaintSectionSeriesMovies":
    "— Jedna položka na propojený film v input.facts.series_movies",
  "settings.refMaintSectionObsSeriesMovies":
    "— Jedna položka na propojený film v input.observations.series_movies.value",
  "settings.refMaintSeriesMovieLinkId": "Identifikátor propojení seriál–film",
  "settings.refMaintSeriesMovieName": "Název propojeného filmu",
  "settings.refMaintSeriesMovieYear":
    "Rok vydání propojeného filmu, je-li známý",
  "settings.refMaintSeriesMovieMonitored":
    "Zda je pro tento propojený film zapnuto získávání",
  "settings.refMaintSeriesMovieHasFile":
    "Zda má tento propojený film soubor na disku",
  "settings.refMaintSeriesMovieTags":
    "Uživatelsky definované štítky tohoto propojeného filmu ze stejného registru jako štítky titulů. Propojený film nese vlastní štítky; nededí je od seriálu.",
  "settings.refMaintTagActionsTitle": "Akce se štítky",
  "settings.refMaintTagActionsNote":
    "add_tags a remove_tags zapisují uživatelské štítky na odpovídající titul. Každý název musí existovat v registru štítků, který spravuje administrátor v Nastavení, a štítky se ukládají malými písmeny — pravidlo odkazující na štítek, který byl mezitím smazán, se pozdrží s tag_not_defined místo zápisu.",
  "settings.refMaintTagOscillationNote":
    "Dvě pravidla, která zapisují opačnou úpravu stejného štítku na stejný titul, pozdrží oba kandidáty s tag_patch_conflict místo toho, aby vyhrálo to, které běželo naposled. Pravidlo, které štítek přidá, a jiné, které jej v pozdějším běhu odebere, takto zachyceno není: jde o chybu v návrhu, která štítek navždy přepíná tam a zpět, takže rozsahy obou pravidel nastavte tak, aby se nemohly shodovat zároveň.",
  "settings.maintenanceTemplateGallery": "Úvodní šablony",
  "settings.maintenanceTemplateGalleryDescription":
    "Načte do editoru hotové porovnávání. Nic se neuloží, dokud jej nezkontrolujete a nestisknete vytvořit.",
  "settings.maintenanceTemplateApply": "Použít šablonu",
  "settings.maintenanceTemplateFacetMovie": "Filmy",
  "settings.maintenanceTemplateFacetShow": "Seriály",
  "settings.maintenanceTemplateGraceBadge": "{{count}}denní ochranná doba",
  "settings.maintenanceTemplateNoGraceBadge": "Bez ochranné doby",
  "settings.maintenanceTemplateDestructiveBadge": "Maže soubory",
  "settings.maintenanceTemplateNeedsProfileBadge": "Vyberte cílový profil",
  "settings.maintenanceTemplateDeadWantedTitle": "Mrtvé požadované položky",
  "settings.maintenanceTemplateDeadWantedDescription":
    "Tituly, které stále sledujete, ale žádný soubor pro ně nikdy nedorazil. Fakty jsou obyčejné hodnoty, takže titul, jehož stav sledování nebo souboru Scryer nevidí, se pozdrží místo shody. Po 30denní ochranné době se rozsah přestane sledovat a cokoli už je na disku, zůstane beze změny.",
  "settings.maintenanceTemplateLibraryAgingTitle": "Stárnutí knihovny",
  "settings.maintenanceTemplateLibraryAgingDescription":
    "Pozor: tato šablona maže titul a jeho soubory. Cokoli má soubor, zestárne a bude odstraněno, pokud nenese štítek „keep“. Nejprve proběhne 180denní ochranná doba a přepínače instance i vlastní zapnutí tohoto pravidla se stále musí shodnout, než se cokoli odstraní.",
  "settings.maintenanceTemplateAddedLongAgoTitle":
    "Přidáno před více než 180 dny",
  "settings.maintenanceTemplateAddedLongAgoDescription":
    "Skutečný výpočet s daty: porovnávání odečte added_at titulu od času vyhodnocení a shoduje se po 180 dnech. Rozsah se přestane sledovat a soubory zůstanou. Titul bez hodnoty added_at se pozdrží místo shody.",
  "settings.maintenanceTemplateOversizedTitle": "Předimenzovaná vydání",
  "settings.maintenanceTemplateOversizedDescription":
    "Tituly, jejichž soubory mají dohromady více než 40 GB. Přesune titul na vámi zvolený profil kvality a znovu vyhledá, jen pokud se profil skutečně změnil — vyberte tento cílový profil v editoru před uložením.",
  "settings.maintenanceTemplateFourKPurgeTitle": "Čištění 4K",
  "settings.maintenanceTemplateFourKPurgeDescription":
    "Jakýkoli titul obsahující soubor 2160p. Po 7denní ochranné době se přesune na vámi zvolený profil kvality a znovu vyhledá, pokud se profil změnil. Vyberte tento cílový profil v editoru před uložením.",
  "settings.maintenanceTemplateRequestedExpiryTitle":
    "Vypršení požadovaných médií",
  "settings.maintenanceTemplateRequestedExpiryDescription":
    "Pozor: tato šablona maže titul a jeho soubory. Tituly, které přišly na základě žádosti, zestárnou po 120 dnech, pokud nenesou štítek „keep“.",
  "settings.maintenanceTemplateDepartedRequesterTitle": "Žadatel, který odešel",
  "settings.maintenanceTemplateDepartedRequesterDescription":
    "Pozor: tato šablona maže titul a jeho soubory. Odpovídá titulům požadovaným jedním konkrétním uživatelem — před uložením upravte zástupný text „departed-user“ v porovnávání na jméno uživatele, které máte na mysli, jinak nebude odpovídat ničemu.",
  "settings.maintenanceTemplateSystemAddedTitle": "Úklid po systémovém přidání",
  "settings.maintenanceTemplateSystemAddedDescription":
    "Tituly, které objevilo prohledávání a ne osoba, a mají soubor. Po 60 dnech se rozsah přestane sledovat a soubory zůstanou tam, kde jsou.",
  "settings.maintenanceTemplateWatchedRequestedTitle":
    "Shlédnuto všemi, kdo požádali",
  "settings.maintenanceTemplateWatchedRequestedDescription":
    "Pozor: tato šablona maže titul a jeho soubory. Odpovídá požadovaným filmům, které už shlédli úplně všichni jejich žadatelé, po 30denní ochranné době. Pouze filmy: Scryer eviduje přehrání na úrovni filmu a epizody, nikdy seriálu, takže seriál se pozdrží místo shody. Pozdrží se také vždy, když žadatel nemá propojený účet mediálního serveru nebo když je poslední synchronizace signálu starší než 48 hodin — neúplný obraz sledování není důvod k mazání.",
  "settings.maintenanceTemplateExpiredLeasesTitle": "Vypršelé nároky žádostí",
  "settings.maintenanceTemplateExpiredLeasesDescription":
    "Pozor: tato šablona maže titul a jeho soubory. Odpovídá titulům, jejichž všechny nároky žádostí vypršely, po 7denní ochranné době. Nárok, který ještě nezačal — titul nebyl importován — se ponechá beze změny, stejně jako titul, který nikdy nárok neměl.",
  "settings.maintenanceTemplateNoProfileTitle": "Bez profilu kvality",
  "settings.maintenanceTemplateNoProfileDescription":
    "Tituly bez přiřazeného profilu kvality. Neprovádí vůbec žádnou akci: slouží jen k sestavení seznamu k prohlédnutí, než se rozhodne, co s ním udělat.",
  "settings.maintenanceTemplateNewestEpisodesTitle":
    "Ponechat 3 nejnovější stažené epizody",
  "settings.maintenanceTemplateNewestEpisodesDescription":
    "Pozor: tato šablona v rozsahu epizody přestane sledovat a smaže stažené epizody s pořadím starším než tři nejnovější podle data vysílání. Speciály, budoucí epizody, chybějící soubory a nevyřešená data pořadí nedostanou, takže se neshodují. Chráněné epizody stále zabírají místa mezi třemi nejnovějšími.",
  "settings.maintenanceTemplateNewestSeasonsTitle":
    "Ponechat 2 nejnovější stažené řady",
  "settings.maintenanceTemplateNewestSeasonsDescription":
    "Pozor: tato šablona v rozsahu řady přestane sledovat a smaže soubory z řad s pořadím starším než dvě nejnovější podle nejnovější způsobilé stažené epizody každé řady. Speciály, budoucí epizody, chybějící soubory a nevyřešená data pořadí nedostanou; chráněné řady stále zabírají místa.",
  "settings.maintenanceTemplateLeftEveryListTitle":
    "Opustily úplně všechny seznamy",
  "settings.maintenanceTemplateLeftEveryListDescription":
    "Tituly, které opustily úplně všechny seznamy, veřejné i osobní, jakmile byl vyřízen každý odchod a poslední je starší než 30 dní. Rozsah se přestane sledovat a soubory zůstanou tam, kde jsou.",
  "settings.maintenanceTemplateListAddedUnwatchedTitle":
    "Přidáno seznamem a nikdy nezhlédnuto",
  "settings.maintenanceTemplateListAddedUnwatchedDescription":
    "Přidá štítek „needs-review“ titulům, které seznam přidal před více než 90 dny a nikdo je nepřehrál. Pozdrží se vždy, když je obraz sledování neúplný. Nic se nemaže.",
  "settings.requestRules": "Pravidla žádostí",
  "settings.noRequestRulesFound": "Zatím žádná pravidla žádostí.",
  "settings.requestRulesDefault":
    "Bez zapnutého pravidla jsou žádosti od členů s automatickým schvalováním schváleny a každá jiná žádost čeká na posouzení.",
  "settings.requestRuleEnabledUpdated": "Aktualizováno: {{name}}.",
  "settings.requestRuleRequesterFieldsHint":
    "Toto pravidlo čte pole žadatele a pro uložení vyžaduje oprávnění Spravovat.",
  "settings.requestRuleLibraries": "Knihovny",
  "settings.requestRuleLibrariesAll": "Všechny knihovny",
  "settings.requestRuleLibrariesHelp":
    "Ponecháním všech políček nezaškrtnutých uplatníte toto pravidlo na žádosti ve všech knihovnách.",
  "settings.requestRuleRevision": "Revize",
  "settings.requestRuleDecisionCount": "Rozhodnutí",
  "settings.requestRuleCopy": "Kopírovat pravidlo",
  "settings.requestRuleCreate": "Vytvořit pravidlo",
  "settings.requestRuleUpdate": "Aktualizovat pravidlo",
  "settings.requestRuleCreateNew": "Nové pravidlo žádosti",
  "settings.requestRuleCreated":
    "Pravidlo žádosti bylo vytvořeno. Zůstává vypnuté, dokud ho nezapnete.",
  "settings.requestRuleUpdated": "Pravidlo žádosti bylo aktualizováno.",
  "settings.requestRuleDeleted": "Pravidlo žádosti {{name}} bylo smazáno.",
  "settings.requestRuleDeleting":
    "Smazat pravidlo žádosti {{name}}? Jeho zaznamenaná rozhodnutí zůstanou zachována.",
  "settings.requestRuleValidationRequired":
    "Pravidlo žádosti potřebuje název a podmínku (matcher).",
  "settings.requestRuleSaveDisabledNote":
    "Nové pravidlo se uloží vypnuté. Zaškrtněte Zapnuto v tabulce, aby mohlo rozhodovat o žádostech.",
  "settings.requestRuleConfirmDiscardTitle": "Zahodit neuložené změny?",
  "settings.requestRuleConfirmDiscardDescription":
    "Podmínka v editoru nebyla uložena. Pokračováním ji nahradíte.",
  "settings.requestRuleRequesters": "Jmenovat lidi v tomto pravidle",
  "settings.requestRuleRequestersHelp":
    "Zaškrtněte účty, kterých se toto pravidlo týká, a použijte je. Podmínka v editoru se přepíše, takže si ji před uložením můžete přečíst.",
  "settings.requestRuleRequestersApply": "Zapsat tato jména do podmínky",
  "settings.requestRuleRequestersUnavailable":
    "Výpis účtů vyžaduje oprávnění, které tato stránka nemá, takže jména je nutné zapsat do podmínky ručně.",
  "settings.requestRuleRequestersNoTarget":
    "Tato podmínka nikoho nejmenuje, takže není kam zapsat vybrané účty. Nejdřív přidejte množinu žadatelů nebo porovnání uživatelského jména.",
  "settings.requestPreviewTitle": "Vyzkoušet toto pravidlo na žádosti",
  "settings.requestPreviewSubtitle":
    "Spustí jednu podmínku na žádosti, kterou popíšete. Nic se neodešle ani nezaznamená.",
  "settings.requestPreviewSource": "Podmínka",
  "settings.requestPreviewSourceStored": "Uložené pravidlo",
  "settings.requestPreviewSourceDraft": "Koncept z editoru",
  "settings.requestPreviewRuleSelect": "Pravidlo",
  "settings.requestPreviewRuleSelectPlaceholder": "Vyberte pravidlo",
  "settings.requestPreviewUser": "Žadatel",
  "settings.requestPreviewUserPlaceholder": "Vyberte účet",
  "settings.requestPreviewLibrary": "Knihovna",
  "settings.requestPreviewLibraryPlaceholder": "Vyberte knihovnu",
  "settings.requestPreviewProfile": "Profil kvality",
  "settings.requestPreviewProfilePlaceholder": "Vyberte profil",
  "settings.requestPreviewMonitor": "Sledování",
  "settings.requestPreviewLease": "Uchovat po dobu",
  "settings.requestPreviewLeaseDays": "Dní",
  "settings.requestPreviewMedia": "Médium",
  "settings.requestPreviewMediaPlaceholder": "Vyhledejte titul",
  "settings.requestPreviewMediaSelected": "Použit {{title}}.",
  "settings.requestPreviewRun": "Spustit náhled",
  "settings.requestPreviewRunning": "Probíhá…",
  "settings.requestPreviewEmpty":
    "Popište žádost a spusťte náhled, abyste viděli, co by toto pravidlo rozhodlo.",
  "settings.requestPreviewMetadataPartial": "Metadata jsou neúplná",
  "settings.requestPreviewShowInput": "Zobrazit vstup, který podmínka viděla",
  "settings.requestPreviewHideInput": "Skrýt vstup",
  "settings.requestPreviewInputUnavailable":
    "Vstupní dokument se nepodařilo znovu načíst.",
  "settings.requestPreviewUndefinedTags":
    "Než je bude možné použít, definujte je v Nastavení › Štítky.",
  "settings.requestDecisionsTitle": "Nedávná rozhodnutí",
  "settings.requestDecisionsSubtitle":
    "Co pravidla žádostí v poslední době rozhodla.",
  "settings.requestDecisionsOutcomeFilter": "Výsledek",
  "settings.requestDecisionsOutcomeAll": "Libovolný výsledek",
  "settings.requestDecisionsColTime": "Vyhodnoceno",
  "settings.requestDecisionsColOutcome": "Výsledek",
  "settings.requestDecisionsColVotes": "Hlasy",
  "settings.requestDecisionsColReasons": "Důvody",
  "settings.requestDecisionsEmpty":
    "Zatím nejsou zaznamenána žádná rozhodnutí.",
  "settings.requestDecisionsPolicySaid": "Pravidla rozhodla: {{outcome}}.",
  "settings.requestDecisionNoVotes": "Žádné pravidlo nehlasovalo.",
  "settings.requestDecisionHeld": "pozdrženo",
  "settings.requestTemplateGallery": "Úvodní pravidla",
  "settings.requestTemplateGalleryDescription":
    "Hotové příklady, které lze načíst do editoru. Použitím se vyplní formulář; stále si ho musíte přečíst, pojmenovat, omezit rozsah a uložit.",
  "settings.requestTemplateGalleryTagNote":
    "Šablona, která vytváří štítek, potřebuje mít tento štítek definovaný v Nastavení › Štítky; nedefinované štítky se při rozhodování o žádosti zahodí.",
  "settings.requestTemplateApply": "Načíst do editoru",
  "settings.requestTemplatePersonTargetedBadge": "Jmenuje lidi",
  "settings.requestTemplateNamedRequestersTitle":
    "Jmenovití žadatelé, média vhodná pro rodiny",
  "settings.requestTemplateNamedRequestersDescription":
    "Schvaluje žádosti od tří jmenovaných lidí, pokud je certifikace vhodná pro rodiny, a označí tu nejvhodnější z nich štítkem. Jména jsou zástupná: před uložením je nahraďte pomocí výběru účtu.",
  "settings.requestTemplateShortLeaseTitle": "Krátké výpůjčky od jedné osoby",
  "settings.requestTemplateShortLeaseDescription":
    "Schvaluje žádosti jedné jmenované osoby, pokud žádá o uchování média na dva týdny nebo méně. Žádost o trvalé uchování nemá počet dní, takže tady se neschvaluje.",
  "settings.requestTemplateLowResolutionTitle": "Žádosti s nízkým rozlišením",
  "settings.requestTemplateLowResolutionDescription":
    "Schvaluje žádosti jedné jmenované osoby, pokud zvolený profil kvality nepřesahuje 720p.",
  "settings.requestTemplateDenyAdultTitle": "Zamítnout obsah pro dospělé",
  "settings.requestTemplateDenyAdultDescription":
    "Zamítne každou žádost o médium, které metadata označují jako obsah pro dospělé, a zaznamená důvod. Nikoho nejmenuje, takže platí pro každého žadatele.",
  "settings.requestTemplateMonthlyQuotaTitle": "Měsíční kvóta schválení",
  "settings.requestTemplateMonthlyQuotaDescription":
    "Pošle žádost k posouzení, jakmile žadatel měl za posledních třicet dní pět schválených žádostí. Nikdy nezamítá; požádá člověka, aby se na to podíval.",
  "settings.requestTemplateHoldApprovalListsTitle":
    "Pozdržet žádosti ze seznamů členů s politikou Schvalování",
  "settings.requestTemplateHoldApprovalListsDescription":
    "Pošle žádost ze seznamu k posouzení, pokud vlastník seznamu má politiku seznamu nastavenou na Schvalování. Ruční žádosti zůstanou nedotčené.",
  "settings.requestTemplateDenyPersonalListAbove1080pTitle":
    "Zamítnout žádosti z osobních seznamů nad 1080p",
  "settings.requestTemplateDenyPersonalListAbove1080pDescription":
    "Zamítne žádost z osobního seznamu člena, pokud jeho profil kvality přesahuje 1080p. Veřejné seznamy a ruční žádosti zůstanou nedotčené.",
  "settings.refReqTitle": "Referenční přehled vstupu žádosti",
  "settings.refReqSubtitle":
    "Každá hodnota, kterou podmínka žádosti umí číst, a co znamená.",
  "settings.refReqIntro":
    "Pravidlo žádosti čte jeden dokument popisující žádost, osobu, která ji podala, knihovnu, na kterou míří, a co o médiu Scryer ví. Fakta jsou obyčejné hodnoty. Fakt, který Scryer nedokázal zjistit, jednoduše chybí, a pravidlo, které čte chybějící fakt, pozdrží žádost pro člověka místo hádání – tuto pojistku sami nepíšete.",
  "settings.refReqOutputTitle": "Co pravidlo žádosti definuje",
  "settings.refReqOutputIntro":
    "Alespoň jedno z toho musí být definováno, jinak pravidlo nemůže nikdy hlasovat a Scryer odmítne ho uložit.",
  "settings.refReqOutputApprove":
    "approve – žádost může být schválena bez toho, aby ji viděl člověk.",
  "settings.refReqOutputDeny":
    "deny – žádost je zamítnuta. Zamítnutí přebije každý jiný hlas.",
  "settings.refReqOutputManual":
    "manual – žádost jde k člověku bez ohledu na to, co jinak hlasovalo.",
  "settings.refReqOutputTags":
    'tags contains "…" – štítky pro médium, uplatní se jen při schválení.',
  "settings.refReqOutputReasons":
    'reasons contains "…" – stabilní kódy, které se zobrazí žadateli.',
  "settings.refReqOutputNoPackage":
    "Nepište řádek package ani import. Scryer oba generuje a před předáním podmínky do tohoto editoru je zase odstraní.",
  "search.keepFor": "Uchovat po dobu",
  "search.keepForForever": "Navždy",
  "search.keepForDays": "Dní: {{count}}",
  "search.keepForCustom": "Zadat počet dní…",
  "search.keepForCustomLabel": "Počet dní k uchování",
  "search.keepForHelp":
    "Jak dlouho Scryer toto médium pro vás podrží. Po uplynutí doby ho může odstranit pravidlo údržby; trvalé uchování nikdy nevyprší.",
  "requests.preflightAutoApprove": "Toto bude schváleno automaticky",
  "requests.preflightManualReview": "Toto vyžaduje schválení",
  "requests.preflightDeny": "Toto by bylo zamítnuto: {{reasons}}",
  "requests.preflightTags": "Bude označeno štítky {{tags}}.",
  "requests.preflightReason": "Důvod: {{reason}}.",
  "requests.preflightMetadataPartial":
    "Scryeru se nepodařilo zjistit vše o tomto titulu, takže rozhodne člověk.",
  "requests.leaseForever": "Uchováno navždy",
  "requests.leaseRequested": "Požadovaných dní: {{count}}",
  "requests.leaseDormant": "Dní od prvního importu: {{count}}",
  "requests.leaseActive": "Uchováno do {{date}}",
  "requests.leaseExpired": "Výpůjčka vypršela",
  "requests.leaseReleased": "Výpůjčka uvolněna",
  "requests.decisionTitle": "Jak bylo rozhodnuto",
  "requests.decisionPolicySaid":
    "Pravidla rozhodla: {{outcome}}; o žádosti rozhodlo něco jiného.",
  "requests.decisionVotes": "Hlasy pravidel",
  "requests.decisionNothingToShow":
    "Žádné pravidlo se k této žádosti nevyjádřilo.",
  "requests.decisionOutcomeAutoApprove": "Schváleno pravidlem",
  "requests.decisionOutcomeManualReview": "Odesláno k posouzení",
  "requests.decisionOutcomeDeny": "Zamítnuto pravidlem",
  "requests.denyReason": "Zamítnuto, protože: {{reasons}}",
  "requests.policyTagsLabel": "Bude označeno štítky",
  "requests.voteApprove": "schválit",
  "requests.voteDeny": "zamítnout",
  "requests.voteManual": "k posouzení",
  "requests.voteAbstain": "bez stanoviska",
  "requests.voteNone": "žádný hlas",
  "requests.fallbackRuleManual":
    "Pravidlo požádalo, aby se na to podíval člověk.",
  "requests.fallbackHeld":
    "Pravidlo nedokázalo rozhodnout, protože hodnota, kterou čte, byla neznámá.",
  "requests.fallbackError":
    "Pravidlo se nepodařilo spustit, takže žádost šla k člověku.",
  "requests.fallbackNoRuleMatched":
    "Žádné pravidlo se na tuto žádost nevztahovalo.",
  "requests.fallbackLibraryPermission":
    "Schváleno oprávněním žadatele Automaticky schvalovat žádosti.",
  "requests.approvedLease": "Uchovat po dobu",
  "requests.approvedLeaseKeepForever": "Navždy, jak bylo požadováno",
  "requests.approvedLeaseKeepRequested": "Požadovaných dní: {{count}}",
  "requests.approvedTags": "Štítky",
  "requests.approvedTagsHelp":
    "Použije se na titul při schválení. Předvyplněno tím, co vytvořila pravidla; přidejte nebo odeberte libovolný štítek, který definoval administrátor v Nastavení > Štítky.",
  "requests.approvedTagsNone": "Nebudou použity žádné štítky.",
  "requests.claimsShow": "Uchování",
  "requests.claimsHide": "Skrýt uchování",
  "requests.claimsEmpty": "Nic tento titul nedrží.",
  "requests.claimExpiresAt": "Drženo do {{date}}",
  "requests.claimDurationDays": "Dní od prvního importu: {{count}}",
  "requests.claimReleasedReason": "Uvolněno: {{reason}}",
  "requests.claimStateDormant": "Čeká se na import",
  "requests.claimStateActive": "Drženo",
  "requests.claimStateExpired": "Vypršelo",
  "requests.claimStateReleased": "Uvolněno",
  "requests.claimStateConverted": "Nahrazeno",
  "requests.claimExtend": "Prodloužit",
  "requests.claimExtendTitle": "Prodloužit toto držení",
  "requests.claimExtendUntil": "Držet do",
  "requests.claimMakePermanent": "Uchovat trvale",
  "requests.claimRelease": "Uvolnit",
  "requests.claimReleaseTitle": "Uvolnit toto držení",
  "requests.claimReleaseReason": "Důvod",
  "requests.claimReleaseReasonPlaceholder": "Proč se toto držení ruší",
  "requests.claimReleaseReasonHelp":
    "Důvod je povinný; zaznamená se k danému držení.",
  "status.requestApprovedClaimFailed":
    "Schváleno {{name}}, ale nic to nedrží: {{error}}",
  "status.claimExtended": "Držení prodlouženo.",
  "status.claimMadePermanent": "Titul je nyní držen trvale.",
  "status.claimReleased": "Držení uvolněno.",
  "settings.refReqSectionInput": "— Žádost, o které se rozhoduje",
  "settings.refReqInputSchemaVersion":
    "Verze tvaru tohoto dokumentu; mění se jen při přidání polí",
  "settings.refReqInputEvaluationTime":
    "Časové razítko RFC3339, kdy byla žádost vyhodnocena",
  "settings.refReqSectionNow":
    "— Čas v okamžiku vyhodnocení, pro pravidla, která závisí na tom, kdy žádost přichází",
  "settings.refReqNowWeekday":
    "Den v týdnu v UTC, malými písmeny a anglicky (monday … sunday)",
  "settings.refReqNowHourUtc": "Hodina dne v UTC, 0 až 23",
  "settings.refReqSectionRequester":
    "— Osoba, která žádost podala. Podmínka, která čte cokoli z tohoto, potřebuje pro uložení oprávnění ke správě oprávnění",
  "settings.refReqRequesterUserId": "Identifikátor žádajícího účtu",
  "settings.refReqRequesterUsername": "Uživatelské jméno žádajícího účtu",
  "settings.refReqRequesterAccountKind":
    "Jak se účet přihlašuje (například lokálně, nebo přes propojeného poskytovatele)",
  "settings.refReqRequesterAppPermissions":
    "Oprávnění, která účet má v rámci celé aplikace",
  "settings.refReqRequesterLibraryPermissions":
    "Oprávnění, která účet má v knihovně, z níž žádost pochází",
  "settings.refReqRequesterLinkedProviders":
    "Mediální servery a poskytovatelé identity, které má účet propojené",
  "settings.refReqRequesterCreatedAt":
    "Kdy byl účet vytvořen; chybí, protože to Scryer zatím nezaznamenává",
  "settings.refReqSectionLibrary": "— Knihovna, na kterou žádost míří",
  "settings.refReqLibraryId": "Identifikátor knihovny",
  "settings.refReqLibraryName": "Název knihovny",
  "settings.refReqLibraryFacet":
    "Druh médií, které knihovna obsahuje (MOVIE, SERIES nebo ANIME)",
  "settings.refReqLibraryIsDefault":
    "True, pokud je toto výchozí knihovna svého druhu",
  "settings.refReqSectionRequest": "— O co bylo požádáno",
  "settings.refReqRequestOrigin":
    "Odkud žádost pochází: manual, public_list nebo personal_list",
  "settings.refReqRequestTitle": "Název, jak zněl v čase žádosti",
  "settings.refReqRequestYear": "Rok vydání, pokud ho poskytovatel znal",
  "settings.refReqRequestExternalIds":
    "Identifikátory poskytovatele; viz input.request.external_ids níže",
  "settings.refReqRequestQualityProfileId":
    "Identifikátor profilu kvality, který žadatel zvolil",
  "settings.refReqRequestQualityProfileName": "Název daného profilu",
  "settings.refReqRequestMonitorType":
    "Sledování, o které žadatel požádal (například ALL_EPISODES)",
  "settings.refReqRequestMonitorSelectionSeasonCount":
    "Kolik řad jmenoval pokročilý výběr sledování",
  "settings.refReqRequestLeaseForever":
    "True, pokud žadatel požádal o trvalé uchování média",
  "settings.refReqRequestLeaseDays":
    "Kolik dní žadatel požádal o uchování; chybí, pokud žádal o navždy",
  "settings.refReqRequestList":
    "Seznam, který žádost podal; viz input.request.list níže. Chybí u ruční žádosti",
  "settings.refReqSectionExternalIds":
    "— Identifikátory poskytovatele na input.request.external_ids",
  "settings.refReqSectionRequestList":
    "— Seznam stojící za žádostí ze seznamu, na input.request.list. Chybí u ruční žádosti a když se seznam nepodařilo načíst",
  "settings.refReqRequestListSubscriptionId": "Identifikátor seznamu",
  "settings.refReqRequestListName":
    "Název seznamu; chybí u osobního seznamu, který se nikdy nepojmenovává",
  "settings.refReqRequestListScope": "public nebo personal",
  "settings.refReqRequestListMode":
    "Co seznam dělá se svými položkami: add, hold, request nebo discover",
  "settings.refReqRequestListProvider": "Poskytovatel seznamu",
  "settings.refReqRequestListOwnerIsRequester":
    "True, pokud je vlastník seznamu totožný s žadatelem",
  "settings.refReqRequestListOwnerListPolicy":
    "Politika seznamu vlastníka: auto, approval nebo none; approval, pokud není uložena žádná",
  "settings.refReqRequestListRank":
    "Pořadí položky na seznamu; chybí, pokud seznam položky neřadí",
  "settings.refReqExternalIdsTmdb":
    "Identifikátor TMDB, pokud ho poskytovatel měl",
  "settings.refReqExternalIdsImdb":
    "Identifikátor IMDb, pokud ho poskytovatel měl",
  "settings.refReqExternalIdsTvdb":
    "Identifikátor TVDB, pokud ho poskytovatel měl",
  "settings.refReqSectionFacts":
    "— Co Scryer ví o médiu a o historii tohoto žadatele. Fakt, který Scryer nedokázal zjistit, chybí, a čtení chybějícího faktu pozdrží žádost pro člověka",
  "settings.refReqFactsAgeRating": "Minimální věk, který certifikace naznačuje",
  "settings.refReqFactsCertifications":
    "Každá certifikace, kterou metadata obsahují; viz input.facts.certifications[] níže",
  "settings.refReqFactsCertificationLabel":
    "Americké certifikační označení (například G, PG, PG-13, R)",
  "settings.refReqFactsCertificationRank":
    "Totéž označení jako číslo, od nejnižšího, aby pravidla mohla porovnávat místo vyjmenovávání označení",
  "settings.refReqFactsCommonsenseRecommended":
    "Zda Common Sense Media doporučuje titul pro jeho věkovou skupinu",
  "settings.refReqFactsGenres": "Názvy žánrů titulu",
  "settings.refReqFactsCanonicalTagKeys":
    "Kanonické klíče štítků Scryer pro tento titul",
  "settings.refReqFactsThemes": "Názvy témat titulu",
  "settings.refReqFactsIsAdult":
    "Zda metadata označují titul jako obsah pro dospělé",
  "settings.refReqFactsRating": "Kombinované hodnocení z metadat",
  "settings.refReqFactsRatingsBySource":
    "Stejná hodnocení podle zdroje, který je poskytl",
  "settings.refReqFactsTmdbVoteAverage": "Průměrné hodnocení TMDB",
  "settings.refReqFactsTmdbVoteCount": "Z kolika hlasů se ten průměr skládá",
  "settings.refReqFactsPopularity": "Skóre popularity od poskytovatele",
  "settings.refReqFactsRuntimeMinutes": "Délka v minutách",
  "settings.refReqFactsOriginalLanguage": "Kód původního jazyka",
  "settings.refReqFactsCountry": "Země původu",
  "settings.refReqFactsNetwork": "Síť, která ho vysílala",
  "settings.refReqFactsStudio": "Studio, které ho vyrobilo",
  "settings.refReqFactsContentStatus":
    "Stav obsahu od poskytovatele (například Continuing, Ended)",
  "settings.refReqFactsReleaseDate": "Datum vydání, u filmů",
  "settings.refReqFactsFirstAired": "Datum prvního vysílání, u seriálů",
  "settings.refReqFactsReleaseAgeDays":
    "Kolik dní uplynulo od vydání nebo prvního vysílání",
  "settings.refReqFactsAwardCount": "Kolik ocenění metadata zaznamenávají",
  "settings.refReqFactsQualityProfileTiers":
    "Názvy úrovní v profilu kvality, který žadatel zvolil",
  "settings.refReqFactsQualityProfileMaxResolution":
    "Nejvyšší svislé rozlišení, které tento profil povoluje, v pixelech",
  "settings.refReqFactsQualityProfileAllowsUpgrades":
    "Zda tento profil pokračuje v upgradech i po prvním stažení",
  "settings.refReqFactsExistsInLibraryIds":
    "Knihovny, které tento titul už obsahují",
  "settings.refReqFactsPreviousRequestCount":
    "Kolikrát o tento titul tento žadatel žádal už dříve",
  "settings.refReqFactsPreviouslyDenied":
    "Zda byla jedna z dřívějších žádostí zamítnuta",
  "settings.refReqFactsPreviouslyApproved":
    "Zda byla jedna z dřívějších žádostí schválena",
  "settings.refReqFactsPendingRequestCount":
    "Kolik žádostí tohoto žadatele stále čeká",
  "settings.refReqFactsApprovedLast30d":
    "Kolik žádostí tohoto žadatele bylo schváleno za posledních třicet dní",
  "settings.refReqFactsDeniedLast30d":
    "Kolik jich bylo zamítnuto za posledních třicet dní",
  "settings.refReqFactsTotalApproved":
    "Kolik žádostí tohoto žadatele bylo kdy schváleno",
  "settings.refReqFactsActiveLeaseCount":
    "Kolik titulů tento žadatel aktuálně drží na výpůjčku",
  "settings.refReqFactsDaysSinceLastRequest":
    "Kolik dní uplynulo od poslední žádosti tohoto žadatele",
  "settings.refReqFactsLibraryTitleCount":
    "Kolik titulů cílová knihovna obsahuje",
  "settings.refReqSectionCertifications":
    "— Jedna položka na certifikaci v input.facts.certifications",
  "settings.refReqCertCountry": "Země, kde byla certifikace vydána",
  "settings.refReqCertValue": "Samotná certifikace (například PG-13)",
  "settings.refReqCertSource": "Odkud certifikace pochází",
  "settings.refReqSectionObservations":
    "— Stejná fakta jako úplné obálky, pro pravidla, která potřebují rozlišit chybějící od neznámého nebo přečíst důvod. Čtení faktu zde vyřazuje daný fakt z automatického pozdržení: přebíráte odpovědnost za trojhodnotovou odpověď.",
  "settings.refReqObsStatus":
    "known, absent nebo unknown. absent znamená, že zdroj odpověděl a hodnota neexistuje; unknown znamená, že to Scryer nedokázal zjistit. Otestujte toto před čtením value – neznámý fakt se nikdy nepřevádí na false, 0 ani prázdný řetězec.",
  "settings.refReqObsObservedAt":
    "Kdy byla hodnota zjištěna, pokud zdroj čas zaznamenal",
  "settings.refReqObsReason":
    "Stabilní strojový kód vysvětlující, proč je fakt neznámý nebo proč chybějící fakt nemá hodnotu",
  "settings.refReqSectionObsCertifications":
    "— Jedna položka na certifikaci v input.observations.certifications.value",
  "settings.refReqFactsAgeRatingValue":
    "Minimální věk daný certifikací; chybí s důvodem no_certification, pokud metadata žádnou neobsahují",
  "settings.refReqFactsCertificationsValue":
    "Jedna položka na certifikaci; viz input.observations.certifications.value[] níže",
  "settings.refReqFactsCertificationLabelValue":
    "Americké certifikační označení; chybí, pokud metadata neobsahují žádnou americkou certifikaci",
  "settings.refReqFactsCertificationRankValue":
    "Totéž označení jako číslo, od nejnižšího; chybí, pokud je označení takové, které Scryer neřadí",
  "settings.refReqFactsCommonsenseRecommendedValue":
    "True, pokud Common Sense Media doporučuje titul pro jeho věkovou skupinu",
  "settings.refReqFactsGenresValue":
    "Názvy žánrů, bez duplicit; prázdný seznam, pokud metadata žádné nejmenují",
  "settings.refReqFactsCanonicalTagKeysValue":
    "Kanonické klíče štítků, bez duplicit",
  "settings.refReqFactsThemesValue": "Názvy témat, bez duplicit",
  "settings.refReqFactsIsAdultValue":
    "True, pokud metadata označují titul jako obsah pro dospělé",
  "settings.refReqFactsRatingValue": "Kombinované hodnocení jako číslo",
  "settings.refReqFactsRatingsBySourceValue":
    "Objekt podle názvu zdroje, kde každá hodnota je hodnocení daného zdroje",
  "settings.refReqFactsTmdbVoteAverageValue":
    "Průměrné hodnocení TMDB jako číslo",
  "settings.refReqFactsTmdbVoteCountValue":
    "Z kolika hlasů se ten průměr skládá",
  "settings.refReqFactsPopularityValue": "Skóre popularity jako číslo",
  "settings.refReqFactsRuntimeMinutesValue": "Délka v celých minutách",
  "settings.refReqFactsOriginalLanguageValue": "Původní jazyk jako kód",
  "settings.refReqFactsCountryValue": "Země původu",
  "settings.refReqFactsNetworkValue": "Název sítě",
  "settings.refReqFactsStudioValue": "Název studia",
  "settings.refReqFactsContentStatusValue":
    "Řetězec stavu obsahu od poskytovatele",
  "settings.refReqFactsReleaseDateValue": "Datum vydání ve formátu RFC3339",
  "settings.refReqFactsFirstAiredValue":
    "Datum prvního vysílání ve formátu RFC3339",
  "settings.refReqFactsReleaseAgeDaysValue":
    "Celé dny od vydání nebo prvního vysílání",
  "settings.refReqFactsAwardCountValue":
    "Počet zaznamenaných ocenění; potvrzená nula, pokud žádná nejsou",
  "settings.refReqFactsQualityProfileTiersValue":
    "Názvy úrovní ve zvoleném profilu, v pořadí profilu",
  "settings.refReqFactsQualityProfileMaxResolutionValue":
    "Nejvyšší svislé rozlišení, které profil povoluje, v pixelech",
  "settings.refReqFactsQualityProfileAllowsUpgradesValue":
    "True, pokud profil pokračuje v upgradech i po prvním stažení",
  "settings.refReqFactsExistsInLibraryIdsValue":
    "Identifikátory knihoven, které už titul obsahují; prázdný seznam, pokud žádná",
  "settings.refReqFactsPreviousRequestCountValue":
    "Počet dřívějších žádostí tohoto žadatele o tento titul",
  "settings.refReqFactsPreviouslyDeniedValue":
    "True, pokud byla jedna z dřívějších žádostí zamítnuta",
  "settings.refReqFactsPreviouslyApprovedValue":
    "True, pokud byla jedna z dřívějších žádostí schválena",
  "settings.refReqFactsPendingRequestCountValue":
    "Počet žádostí tohoto žadatele, které stále čekají na rozhodnutí",
  "settings.refReqFactsApprovedLast30dValue":
    "Počet schválených za posledních třicet dní, měřeno vůči času vyhodnocení",
  "settings.refReqFactsDeniedLast30dValue":
    "Počet zamítnutých za posledních třicet dní, měřeno vůči času vyhodnocení",
  "settings.refReqFactsTotalApprovedValue":
    "Počet žádostí tohoto žadatele, které byly kdy schváleny",
  "settings.refReqFactsActiveLeaseCountValue":
    "Počet titulů, které tento žadatel aktuálně drží na výpůjčku, včetně spících výpůjček",
  "settings.refReqFactsDaysSinceLastRequestValue":
    "Celé dny od předchozí žádosti žadatele; chybí s důvodem no_previous_request, pokud je toto jeho první",
  "settings.refReqFactsLibraryTitleCountValue":
    "Počet titulů v cílové knihovně",
  "move.dialogTitle": "Přesunout tituly",
  "move.dialogDescription":
    "Před jakýmkoli přesunem zkontrolujte plán. Vybraných titulů: {{count}}.",
  "move.destinationLibrary": "Cílová knihovna",
  "move.destinationRoot": "Cílový kořen",
  "move.destinationRootPlaceholder": "Vyberte kořen",
  "move.actionHeading": "Přesunout do…",
  "move.actionDescription":
    "Přesunout soubory tohoto titulu do jiné kořenové složky nebo do jiné knihovny.",
  "move.actionButton": "Přesunout do…",
  "move.wizardKindDescription": "Vyberte, kam by měl tento titul jít.",
  "move.kindRootHeading": "Přesunout do jiného kořene",
  "move.kindRootHelp":
    "Ponechte titul v této knihovně a přesuňte jeho soubory do jedné z jejích dalších kořenových složek.",
  "move.kindLibraryHeading": "Přesunout do jiné knihovny",
  "move.kindLibraryHelp":
    "Přesunout titul do jiné knihovny, do jednoho z kořenů dané knihovny.",
  "move.noOtherRoots":
    "Tato knihovna nemá žádnou jinou kořenovou složku, kam přesunout. Přidejte ji do knihovny, nebo přesuňte titul do jiné knihovny.",
  "move.back": "Zpět",
  "move.next": "Další",
  "move.destinationRootHelp":
    "Změna cíle otevře průvodce přesunem místo uložení titulu.",
  "move.destinationNoSelection": "Nejsou vybrány žádné tituly.",
  "move.destinationMixedSourceLibraries":
    "Výběr zahrnuje více knihoven; přesouvejte po jedné knihovně.",
  "move.destinationCrossLibraryNotice":
    "Tyto tituly opustí svou současnou knihovnu a stanou se součástí {{library}}.",
  "move.destinationLibraryInMoveWorkflow":
    "Vyberte jinou knihovnu v průvodci přesunem, po výběru cílového kořene.",
  "move.transferIntoLibrary": "Přesouvá se do {{library}}.",
  "move.sameNameWarning":
    "{{library}} už má titul s názvem „{{title}}“. Scryer tituly podle jména nikdy neslučuje, takže po tomto přesunu budou existovat dva tituly se stejným názvem.",
  "move.ambiguousDestinationHelp":
    "Několik titulů v cílové knihovně sdílí identitu tohoto titulu a Scryer mezi nimi nebude hádat. Nejdřív vyřešte identitu v cílové knihovně, pak tento titul přesuňte znovu.",
  "move.ambiguousCandidateShared": "Sdílí {{identities}}",
  "move.mergeStatement":
    "Slučuje se s „{{title}}“. Oba se stanou jedním titulem: identita tohoto titulu se pohltí a vyhrává nastavení titulu „{{title}}“.",
  "move.mergeSummaryHeading": "Co toto sloučení udělá",
  "move.mergeNoDetails": "Toto sloučení nepřenáší nic dalšího.",
  "move.mergeBlockedHeading":
    "Toto sloučení nelze provést: tyto záznamy nelze přenést.",
  "move.mergeCarried":
    "Přenese se mediálních souborů: {{files}} a záznamů historie: {{history}}.",
  "move.mergeDropped":
    "S tímto titulem zanikne dalších záznamů (štítky, žádosti, stav získávání): {{count}}.",
  "move.mergeRoleChangesHeading":
    "Souborů, které se stanou dalšími kopiemi: {{demotions}}",
  "move.mergeRoleChangesHeadingPlain": "Role souborů",
  "move.mergeRoleGroup.DESTINATION_PRIMARY_RETAINED":
    "„{{title}}“ už má pro tyto epizody hlavní soubor. Příchozí soubory se ponechají jako další kopie a nic stávajícího se nenahradí.",
  "move.mergeRoleGroup.DESTINATION_PRIMARY_RETAINED_TITLE":
    "„{{title}}“ už má hlavní soubor. Příchozí soubor se ponechá jako další kopie a nic stávajícího se nenahradí.",
  "move.mergeRoleGroup.SOURCE_PRIMARY_ALREADY_CLAIMED":
    "Jiný soubor v tomto přesunu už je pro tyto epizody hlavní, takže tyto se ponechají jako další kopie.",
  "move.mergeRoleGroup.SOURCE_PRIMARY_ALREADY_CLAIMED_TITLE":
    "Jiný soubor v tomto přesunu už je hlavní, takže tento se ponechá jako další kopie.",
  "move.mergeRoleGroup.COLLAPSED_SOURCE_EPISODES":
    "Tyto soubory jsou zde uvedeny pod dvěma epizodami, které jsou v cíli jednou epizodou. Tam je každý uveden jednou.",
  "move.mergeRoleUnnamedFile": "soubor {{id}}",
  "move.operationTransferNote": "Tyto tituly se přesouvají do jiné knihovny.",
  "move.modeHeading": "Jak se tam mají soubory dostat?",
  "move.mode.MOVE_WITH_SCRYER": "Přesunout pomocí Scryeru",
  "move.mode.FILES_ALREADY_THERE": "Soubory jsou již tam",
  "move.modeHelp.MOVE_WITH_SCRYER":
    "Scryer soubory přesune nebo zkopíruje a každý z nich ověří dřív, než se změní katalog.",
  "move.modeHelp.FILES_ALREADY_THERE":
    "Scryer prohledá cíl a převezme soubory, které jste přesunuli sami.",
  "move.catalogOnlyFastPath":
    "Tyto tituly nemají žádné soubory na disku, takže jde jen o změnu v katalogu: nic se nepřesouvá a žádný režim není potřeba.",
  "move.selectDestinationPrompt": "Vyberte cílový kořen pro zobrazení plánu.",
  "move.previewLoading": "Shromažďují se všechny informace...",
  "move.gatheringInfo": "Shromažďují se všechny informace...",
  "move.methodManaged": "Scryer přesune soubory",
  "move.methodManual": "Soubory přesunu sám/sama",
  "move.manualLoading": "Načítají se pokyny ke složce...",
  "move.manualInstructions":
    "Přesuňte soubory a pak se vraťte a klikněte na Další.",
  "move.changeDestination": "Změnit cíl",
  "move.manualMappingChanged":
    "Umístění v katalogu se změnila. Před opětovným kliknutím na Další zkontrolujte aktualizované složky.",
  "move.manualStructure":
    "Použijte cílové složky níže, zachovejte názvy souborů a relativní adresáře. Další aktualizuje katalog a spoléhá na to, že jste soubory přesunuli; Scryer je nebude kontrolovat ani měnit.",
  "move.manualSource": "Aktuální složka",
  "move.manualDestination": "Přesunout do složky",
  "move.retryInstructions": "Znovu načíst pokyny",
  "move.previewFailed": "Náhled přesunu se nezdařil.",
  "move.planChanged":
    "Plán se od vašeho náhledu změnil. Před potvrzením zkontrolujte nový plán.",
  "move.rePreview": "Znovu zobrazit náhled",
  "move.summaryTitles": "Tituly",
  "move.summaryFiles": "Soubory",
  "move.summarySize": "Celková velikost",
  "move.sampledItems": "ukázka",
  "move.blockedHeading": "Titulů, které se zatím nemohou přesunout: {{count}}",
  "move.blockedHelp":
    "Zrušte jejich výběr, aby se přesunuly ostatní, nebo počkejte, až jejich práce skončí.",
  "move.deselect": "Zrušit výběr",
  "move.deselectedCount": "Titulů odebraných z tohoto přesunu: {{count}}.",
  "move.restoreSelection": "Obnovit výběr",
  "move.titleFileSummary": "Souborů: {{files}} · {{size}}",
  "move.freeSpaceRequired":
    "Na cíli je potřeba {{required}}; volno {{available}}.",
  "move.freeSpaceUnknown": "neznámé",
  "move.freeSpaceInsufficient": "Cíl nemá dost volného místa.",
  "move.startRefusedNoSpace":
    "Cíl nemá pro tento přesun dost volného místa. Uvolněte místo nebo přesuňte méně titulů a zkuste to znovu.",
  "move.freeSpaceNotProbed": "Objem cíle se nepodařilo změřit.",
  "move.freeSpaceRecycleOtherVolume":
    "Koš je na jiném svazku a potřebuje {{required}}.",
  "move.verificationStatementFull":
    "Všechny soubory ({{files}}, {{bytes}}) budou plně přečteny zpět a porovnány.",
  "move.verificationStatementQuick":
    "Všechny soubory ({{files}}, {{bytes}}) budou zkontrolovány podle začátku, konce a velikosti.",
  "move.verificationNotApplicable":
    "Žádné soubory se nepřesouvají, takže není co ověřovat.",
  "move.verificationStampFull": "ověřeno (úplně)",
  "move.verificationStampQuick": "ověřeno (rychle)",
  "move.verificationFallbackCount":
    "Souborů přepnutých na rychlou kontrolu: {{count}}",
  "move.typedConfirmationPrompt": "Pro potvrzení napište frázi",
  "move.confirm": "Spustit přesun",
  "move.startFailed": "Spuštění přesunu se nezdařilo.",
  "move.operationLoading": "Načítá se operace…",
  "move.operationLoadFailed": "Načtení operace se nezdařilo.",
  "move.operationMissing": "Tato operace už neexistuje.",
  "move.operationDismiss": "Zavřít",
  "move.operationProgress": "Tituly {{titles}} · Soubory {{files}} · {{bytes}}",
  "move.counterMerges": "Sloučeno",
  "move.counterDedups": "Deduplikováno",
  "move.counterRenames": "Přejmenováno",
  "move.counterNoOps": "Beze změny",
  "move.counterUnresolved": "Nevyřešeno",
  "move.counterBlocked": "Blokováno",
  "move.checkpointsHeading": "Tituly",
  "move.checkpointsPending":
    "Tituly se zde objeví, jakmile vstoupí do operace.",
  "move.checkpointsShowAll": "Zobrazit všechny tituly ({{count}})",
  "move.checkpointsShowLess": "Zobrazit méně titulů",
  "move.transferShowMore": "Zobrazit více",
  "move.transferFilesForTitle": "Zobrazit soubory pro {{title}}",
  "move.transferFilePath": "Soubor",
  "move.transferCopyProgress": "Umístění",
  "move.transferPlacementAwaitingComparison": "Čeká se na porovnání",
  "move.transferVerifyProgress": "Ověřování",
  "move.transferVerificationBytes": "Ověřeno {{done}} / {{total}}",
  "move.transferNoFiles": "Pro tento titul nejsou žádné přenosy souborů.",
  "move.transferFileState.QUEUED": "Ve frontě",
  "move.transferFileState.WAITING_FOR_STORAGE":
    "Čeká se na úložiště · kontrola každé 2 min",
  "move.transferFileState.DUPLICATE": "Duplicita (bez kopie)",
  "move.transferFileState.MOVING": "Přesouvá se",
  "move.transferFileState.VERIFYING": "Ověřuje se",
  "move.transferFileState.COMPARING": "Porovnávají se soubory",
  "move.transferFileState.DONE": "Hotovo",
  "move.transferFileState.FAILED": "Chyba",
  "move.transferFileState.BLOCKED": "Blokováno",
  "move.transferFileState.CANCELED": "Zrušeno",
  "move.transferFileState.NOT_PROCESSED": "Nezpracováno",
  "move.transferShowLess": "Zobrazit méně",
  "move.transferOverallProgress": "Celkový průběh",
  "move.transferEstimating": "Odhaduje se…",
  "move.transferDisconnected": "Znovu se připojuje…",
  "move.transferEta": "Zbývá přibližně {{minutes}} min",
  "move.transferColumnTitle": "Titul",
  "move.transferColumnStatus": "Stav",
  "move.transferColumnFiles": "Soubory hotovo/celkem",
  "move.transferColumnCurrentFile": "Aktuální soubor",
  "move.transferColumnProgress": "Průběh",
  "move.transferRange": "{{from}}–{{to}} z {{total}}",
  "move.transferPrevious": "Předchozí",
  "move.transferNext": "Další",
  "move.transferException": "Zobrazit výjimku pro {{title}}",
  "move.transferActive": "{{copying}} se přesouvá · {{verifying}} se ověřuje",
  "move.transferDetailPending": "Žádné další podrobnosti nejsou k dispozici.",
  "move.checkpointClass": "Klasifikováno jako",
  "move.checkpointFrom": "Odkud",
  "move.checkpointTo": "Kam",
  "move.checkpointMergedInto": "Sloučeno do",
  "move.checkpointVerified": "Ověřeno",
  "move.checkpointVerifiedValue": "Souborů: {{files}} · {{bytes}}",
  "move.checkpointDetail": "Podrobnosti",
  "move.assetsHeading": "Přejmenované a deduplikované soubory",
  "move.assetRenamedAs": "{{from}} → {{to}}",
  "move.assetDeduplicatedAgainst": "{{from}} deduplikováno vůči {{to}}",
  "move.assetUnknownCounterpart": "nepojmenovaný soubor",
  "move.assetProvenance": "(z {{library}})",
  "move.assetStateDone": "Hotovo",
  "move.assetStatePlanned": "Plánováno",
  "move.cancelAction": "Zrušit operaci",
  "move.cancelRequested":
    "Zrušení požadováno; probíhající práce se bezpečně dokončí.",
  "move.cancelNotPossible": "Tato operace už skončila.",
  "move.cancelFailed": "Požadavek na zrušení se nezdařil.",
  "move.cancelDraining": "Dokončuje se probíhající práce",
  "move.resumeAction": "Obnovit",
  "move.resumeRequested":
    "Obnovuje se od posledního ověřeného kontrolního bodu.",
  "move.resumeNotPossible": "Tuto operaci nelze obnovit.",
  "move.resumeFailed": "Obnovení operace se nezdařilo.",
  "move.retryAction": "Zkusit znovu",
  "move.retryRequested":
    "Zkouší se znovu od posledního ověřeného kontrolního bodu.",
  "move.retryNotPossible": "Tuto operaci nelze zkusit znovu.",
  "move.retryFailed": "Opakování operace se nezdařilo.",
  "move.abandonAction": "Opustit",
  "move.abandonRequested":
    "Opuštěno. Jeho tituly a kořeny se uvolní; dokončené tituly zůstanou nezměněné a Zkusit znovu přesune zbytek, jakmile se úložiště vrátí.",
  "move.abandonNotPossible": "Tato operace se již zastavila.",
  "move.abandonFailed": "Opuštění operace se nezdařilo.",
  "move.stalledNoRunner": "Žádný běžec",
  "move.stalledGuidance":
    "Na této operaci nepracuje žádný běžec. Pokud se úložiště vrátilo, stiskněte Obnovit. Pokud se nevrátí, stiskněte Opustit, abyste uvolnili jeho tituly a kořeny; poté bude k dispozici Zkusit znovu.",
  "move.planAgainAction": "Naplánovat znovu",
  "move.planAgainTitleAction": "Naplánovat tento titul znovu",
  "move.planAgainFailed": "Dialog pro přesun se nepodařilo otevřít.",
  "move.planAgainCatalogBlocked":
    "Titulů mimo Naplánovat znovu: {{count}}. Po ověření souborů se u nich nedokončila aktualizace katalogu; Zkusit znovu ji dokončí.",
  "move.reason.ownership_conflict":
    "Některé z těchto titulů nebo kořenů stále vlastní jiná operace. Počkejte, až skončí (nebo ji zrušte), a zkuste to znovu.",
  "move.reason.storage_error":
    "Soubor se nepodařilo přečíst nebo zapsat. Ze zdroje nebylo nic odebráno. Opravte problém s úložištěm (volné místo, oprávnění, připojení disku) a zkuste to znovu; již ověřené soubory se znovu nekopírují.",
  "move.reason.verification_mismatch":
    "Zkopírovaný soubor neodpovídal zdroji. Zdroj zůstává nedotčen a kopii nelze považovat za důvěryhodnou. Zkuste to znovu; pokud stejný soubor selže znovu, zkontrolujte cílový disk.",
  "move.reason.stale_plan":
    "Po pořízení náhledu se v plánu něco změnilo. Dokončené tituly jsou v pořádku. Naplánujte přesun znovu, abyste pokračovali s aktuálním náhledem.",
  "move.reason.merge_unavailable":
    "Tento titul se již nemůže sloučit se svým cílem. Opravte cílový titul (nebo ho odeberte z výběru) a naplánujte přesun znovu.",
  "move.reason.catalog_write_failed":
    "Soubory byly zkopírovány a ověřeny, ale aktualizace katalogu selhala. Ze zdroje nebylo nic odebráno. Zkuste to znovu, aby se aktualizace katalogu dokončila; pro tento titul neplánujte nový přesun, dokud se nezdaří.",
  "move.reason.source_cleanup_failed":
    "Přesun se dokončil, ale starou kopii se nepodařilo odebrat. Zkuste to znovu, nebo potvrďte, že titul přehrává z nového umístění, a starou složku smažte sami.",
  "move.reason.stranded":
    "Tato operace byla přijata, ale nikdy se nespustila. Zkuste to znovu, nebo naplánujte přesun znovu.",
  "move.reason.canceled":
    "Zrušeno na hranici titulu. Dokončené tituly zůstávají beze změny; zkuste to znovu a dokončete zbytek.",
  "move.reason.abandoned":
    "Opuštěno. Dokončené tituly zůstávají beze změny. Zkuste to znovu, jakmile bude úložiště zase dostupné.",
  "move.reason.root_unavailable":
    "Kořen, který tato operace potřebuje, není dostupný. Pokračujte, až bude znovu připojen, nebo operaci opusťte a uvolněte její tituly.",
  "move.class.ROOT_MOVE": "Přesun do cílového kořene",
  "move.class.CROSS_LIBRARY_TRANSFER": "Přesun do jiné knihovny",
  "move.class.CATALOG_ONLY": "Pouze katalog (žádné soubory na disku)",
  "move.class.NO_OP": "Již v cíli",
  "move.class.NEEDS_RESOLUTION": "Vyžaduje rozhodnutí",
  "move.class.INCOMPATIBLE": "Tam nelze přejít",
  "move.planKind.MOVE": "Přesuny",
  "move.planKind.RENAME": "Přejmenování",
  "move.planKind.MERGE": "Sloučení",
  "move.planKind.DEDUP": "Může se sloučit s identickými soubory",
  "move.planKind.CATALOG_CHANGE": "Změny katalogu",
  "move.planKind.ROLE_CHANGE": "Změny role",
  "move.planKind.NO_OP": "Beze změny",
  "move.planKind.BLOCKED": "Blokováno",
  "move.planKind.UNMANAGED_CONTENT": "Nespravovaný obsah",
  "move.planKind.WARNING": "Upozornění",
  "move.operationState.QUEUED": "Ve frontě",
  "move.operationState.PREPARING": "Příprava",
  "move.operationState.MOVING": "Přesouvá se",
  "move.operationState.VERIFYING": "Ověřuje se",
  "move.transferDismiss": "Zavřít",
  "move.operationState.RECONCILING": "Dokončuje se",
  "move.operationState.CLEANING_UP": "Dokončuje se",
  "move.operationState.COMPLETED": "Dokončeno",
  "move.operationState.COMPLETED_WITH_WARNINGS": "Dokončeno s upozorněními",
  "move.operationState.CANCELED": "Zrušeno",
  "move.operationState.FAILED": "Selhalo",
  "move.checkpointState.PENDING": "Čeká",
  "move.checkpointState.MOVING": "Přesouvá se",
  "move.checkpointState.VERIFYING": "Ověřuje se",
  "move.checkpointState.RECONCILING": "Dokončuje se",
  "move.checkpointState.CLEANING_UP": "Dokončuje se",
  "move.checkpointState.COMPLETED": "Dokončeno",
  "move.checkpointState.COMPLETED_WITH_WARNINGS": "Dokončeno s upozorněními",
  "move.checkpointState.SKIPPED": "Přeskočeno",
  "move.checkpointState.BLOCKED": "Blokováno",
  "move.checkpointState.FAILED": "Selhalo",
  "move.operationType.TITLE_FOLDER_RECOVERY":
    "Obnova složky titulu (originály zachovány)",
  "move.operationType.FOLDER_REASSIGNMENT": "Přiřazení složky",
  "move.operationType.ROOT_MOVE": "Přesun kořene",
  "move.operationType.ROOT_CHANGE": "Změna kořene",
  "move.operationType.ROOT_CONSOLIDATION": "Sloučení kořenů",
  "move.operationType.CROSS_LIBRARY_TRANSFER": "Přesun mezi knihovnami",
  "move.toastPreparing": "Příprava…",
  "move.toastTransferred": "Přesunuto",
  "move.toastTitlesProgress": "Tituly: {{current}} / {{total}}",
  "move.toastDoneSubtitle": "Přesun dokončen · titulů: {{count}}",
  "move.toastWarningsSubtitle": "Dokončeno s upozorněními · titulů: {{count}}",
  "move.toastWarningsChip":
    "Některé tituly je potřeba zkontrolovat v Aktivitě.",
  "move.toastFailedSubtitle": "Přesun selhal",
  "move.toastCanceledSubtitle": "Přesun zrušen",
  "move.startedHeading": "Přesun byl zahájen. Sledujte ho v Aktivitě.",
  "move.viewInActivity": "Zobrazit v Aktivitě",
  "rootChange.rowAction": "Změnit kořen",
  "rootChange.unavailableWhileUnsaved":
    "Nejprve uložte nebo zahoďte neuložené úpravy kořene.",
  "rootChange.dialogTitle": "Změnit kořen",
  "rootChange.dialogDescription":
    "Vše na {path} se přesune. Operace nad celým kořenem nemůže žádný titul vynechat.",
  "rootChange.destinationHeading": "Kam tento kořen přesunout",
  "rootChange.destinationNewPath": "Nová cesta",
  "rootChange.destinationNewPathHelp":
    "Kořen si zachová svou identitu i přiřazení titulů; mění se jen jeho cesta.",
  "rootChange.destinationExistingRoot": "Jiný kořen této knihovny",
  "rootChange.destinationExistingRootHelp":
    "Tento kořen se sloučí do vybraného kořene a jeho konfigurace zaniká.",
  "rootChange.destinationNoOtherRoots":
    "Tato knihovna nemá žádný jiný kořen, do kterého by se dalo sloučit.",
  "rootChange.destinationPathPlaceholder": "/mnt/new-disk/movies",
  "rootChange.planChanged":
    "Plán se od náhledu změnil. Před potvrzením si vytvořte nový náhled.",
  "rootChange.previewFailed":
    "Náhled této změny kořene se nepodařilo zobrazit.",
  "rootChange.previewing": "Čte se celý kořen.",
  "rootChange.preview": "Náhled",
  "rootChange.confirm": "Spustit",
  "rootChange.startFailed": "Tuto změnu kořene se nepodařilo spustit.",
  "rootChange.startedHeading":
    "Změna kořene byla zahájena. Sledujte ji v Aktivitě.",
  "rootChange.viewInActivity": "Zobrazit v Aktivitě",
  "rootChange.identityKeepsRoot":
    "Tento kořen si zachová svou identitu i všech svých {titles} přiřazení titulů.",
  "rootChange.identityKeepsDefault": "Zůstane výchozím kořenem této knihovny.",
  "rootChange.identityLosesDefault":
    "Už nebude výchozím kořenem této knihovny.",
  "rootChange.consolidationStatement":
    "Všech {titles} titulů na tomto kořeni se přesune na vybraný kořen.",
  "rootChange.consolidationRetiresConfiguration":
    "Konfigurace zdrojového kořene zanikne, jakmile vše doputuje na místo a dokončí se veškeré odstraňování do koše.",
  "rootChange.defaultTransfers":
    "Cílový kořen se stane výchozím pro tuto knihovnu, takže nový obsah bude přibývat tam.",
  "rootChange.defaultStays": "Výchozí kořen této knihovny se nemění.",
  "rootChange.accountingHeading": "Každý titul na tomto kořeni",
  "rootChange.accountingAssigned": "Přiřazeno",
  "rootChange.accountingRelocating": "Přesouvá se",
  "rootChange.accountingCatalogOnly": "Pouze katalog",
  "rootChange.accountingBlocked": "Blokováno",
  "rootChange.accountingDoesNotClose":
    "Počty nesouhlasí s celkovým součtem. Tento plán nepotvrzujte.",
  "rootChange.blockedTitlesHeading":
    "{count} titulů se nemůže do této operace zapojit.",
  "rootChange.blockedTitlesNoExclude":
    "Opravte je před zahájením. Operace nad celým kořenem nemůže žádný titul vynechat.",
  "rootChange.groupsHeading": "Co toto sloučení provede",
  "rootChange.group.movingIntoUnusedFolders": "Do volných složek",
  "rootChange.group.mergingWithDestinationTitles": "Slučuje se",
  "rootChange.group.folderNameCollisions": "Název složky je obsazen",
  "rootChange.group.mediaCollisions": "Název média je obsazen",
  "rootChange.group.dedupEligibleFiles": "Mohou být identické",
  "rootChange.group.companionCollisions":
    "Název doprovodného souboru je obsazen",
  "rootChange.group.untrackedSourceEntries": "Nesledováno u zdroje",
  "rootChange.renamedFoldersHeading": "Složky, kterým se změní název",
  "rootChange.renamedFoldersSampled": "Zde je uveden jen výpis prvních složek.",
  "rootChange.contentHeading": "Co se na tomto kořeni nachází",
  "rootChange.contentManaged": "Sledovaná média",
  "rootChange.contentCompanions": "Doprovodné soubory",
  "rootChange.contentUnknown": "Nevysvětleno",
  "rootChange.contentSampled": "Zde je uveden jen výpis prvních položek.",
  "rootChange.unknownContentHelp":
    "{bytes} obsahu nic nesleduje. Nikdy se nepřesune ani nesmaže a nadále drží staré umístění obsazené.",
  "rootChange.retirementHeading": "Staré umístění poté",
  "rootChange.retirementEmptyDirectoriesOnly":
    "Odebrány budou jen prázdné adresáře: {count} z nich.",
  "rootChange.retirementVerificationFirst":
    "U zdroje se nic neodebere, dokud se neověří kopie v cíli.",
  "rootChange.retirementRetainedHeading": "Ponecháno na místě i s obsahem:",
  "rootChange.retirementBlockedHeading":
    "Staré umístění zůstane, dokud se nevyřeší toto:",
  "rootChange.retirementPermitted":
    "Nic nebrání tomu, aby staré umístění zaniklo.",
  "rootChange.verificationStatement":
    "{files} souborů ({bytes}) se v cíli plně ověří, než se cokoli u zdroje odstraní do koše.",
  "rootChange.insufficientSpace": "Cíl nemá dostatek místa – chybí {required}.",
  "rootChange.typedConfirmationPrompt": "Pro potvrzení napište danou frázi",
  "rootChange.reason.root_identity_retained":
    "Kořen si zachová svou identitu; mění se jen jeho cesta.",
  "rootChange.reason.catalog_only_root_change":
    "Žádné sledované soubory, jde tedy jen o přeřazení v katalogu.",
  "rootChange.reason.title_blocked_for_root_change":
    "Tento titul se zatím nemůže do změny kořene zapojit.",
  "rootChange.reason.unknown_root_content":
    "Obsah na kořeni, za který žádný titul neodpovídá.",
  "rootChange.reason.source_retirement_blocked":
    "Staré umístění zatím nelze zrušit.",
  "rootChange.reason.file_outside_title_folder":
    "Sledovaný soubor, který leží mimo složku svého titulu.",
  "rootChange.reason.hardlinked_source":
    "Zdrojový soubor má další pevné odkazy, které se při přesunu nepřenesou.",
  "rootChange.reason.roots_consolidated":
    "Každý titul na tomto kořeni se přesune na cílový kořen.",
  "rootChange.reason.default_root_transferred":
    "Cílový kořen se stane výchozím pro tuto knihovnu.",
  "rootChange.reason.moves_into_unused_folder":
    "Přesune se do složky, kterou nic neobsazuje.",
  "rootChange.reason.merges_with_destination_title":
    "Slučuje se s cílovým titulem, se kterým sdílí identitu.",
  "rootChange.reason.folder_name_uniqued":
    "Název složky je v cíli obsazen, proto se tato přejmenuje.",
  "rootChange.reason.catalog_only_consolidation":
    "Žádné sledované soubory, jde tedy jen o přeřazení v katalogu.",
  "rootChange.reason.destination_folder_exists": "Cílová složka už existuje.",
  "rootChange.retirementBlocker.blocked_titles":
    "Některé tituly se nemohou do operace zapojit.",
  "rootChange.retirementBlocker.unexplained_source_content":
    "Obsah u zdroje, za který nic neodpovídá.",
  "rootChange.refusal.root_change_path_not_absolute":
    "Zadejte absolutní cestu.",
  "rootChange.refusal.root_change_paths_overlap":
    "Nová cesta leží uvnitř staré, nebo naopak.",
  "rootChange.refusal.root_change_source_root_is_symlink":
    "Tento kořen je symbolický odkaz. Nejprve ho nasměrujte na skutečný adresář.",
  "rootChange.refusal.root_change_source_root_unavailable":
    "Tento kořen teď není čitelný, takže jeho obsah nelze naplánovat.",
  "rootChange.refusal.root_change_destination_not_empty":
    "Nová cesta už obsahuje data. Zvolte prázdný nebo nový adresář.",
  "rootChange.refusal.root_change_destination_parent_missing":
    "Nadřazený adresář nové cesty neexistuje.",
  "rootChange.refusal.root_change_destination_is_configured_root":
    "Tato cesta je už nastavena jako kořen jiné knihovny.",
  "rootChange.refusal.root_change_destination_conflicts_with_recycle_bin":
    "Tato cesta se překrývá s vlastním košem: nesmí ho obsahovat ani v něm ležet.",
  "rootChange.refusal.root_change_mode_not_supported":
    "Změna kořene vždy soubory přesune; možnost „soubory už tam jsou“ zde není nabízena.",
  "rootChange.refusal.root_consolidation_path_not_absolute":
    "Zadejte absolutní cestu.",
  "rootChange.refusal.root_consolidation_same_root":
    "Kořen nelze sloučit sám se sebou.",
  "rootChange.refusal.root_consolidation_paths_overlap":
    "Jeden z těchto kořenů leží uvnitř druhého.",
  "rootChange.refusal.root_consolidation_source_root_is_symlink":
    "Tento kořen je symbolický odkaz. Nejprve ho nasměrujte na skutečný adresář.",
  "rootChange.refusal.root_consolidation_source_root_unavailable":
    "Tento kořen teď není čitelný, takže jeho obsah nelze naplánovat.",
  "rootChange.refusal.root_consolidation_destination_root_unavailable":
    "Cílový kořen teď není čitelný, takže jeho stávající obsah nelze zohlednit.",
  "rootChange.refusal.root_consolidation_mode_not_supported":
    "Sloučení vždy soubory přesune; možnost „soubory už tam jsou“ zde není nabízena.",
  "indexerSearch.searchWithThisIndexer": "Hledat s tímto indexerem",
  "indexerSearch.queryPlaceholder": "Hledat vydání ve všech indexerech…",
  "indexerSearch.kindLabel": "Druh",
  "indexerSearch.kind.movie": "Film",
  "indexerSearch.kind.series": "Seriál",
  "indexerSearch.kind.anime": "Anime",
  "indexerSearch.kind.raw": "Surové",
  "indexerSearch.scope.indexers": "Indexery",
  "indexerSearch.scope.indexersAll": "Všechny ({{count}})",
  "indexerSearch.scope.indexersSome": "Vybráno: {{count}}",
  "indexerSearch.scope.indexersAllOption": "Všechny indexery",
  "indexerSearch.scope.categories": "Kategorie",
  "indexerSearch.scope.categoriesDefault": "Všechny kategorie",
  "indexerSearch.scope.categoriesHelp":
    "ID kategorií Newznab oddělená čárkami. Ponechte prázdné pro výchozí hodnoty podle vybraného druhu.",
  "indexerSearch.advanced": "Pokročilé",
  "indexerSearch.advanced.minSize": "Min. velikost",
  "indexerSearch.advanced.maxSize": "Max. velikost",
  "indexerSearch.advanced.minSeeders": "Min. počet seederů",
  "indexerSearch.advanced.maxAge": "Max. stáří",
  "indexerSearch.advanced.limit": "Limit výsledků",
  "indexerSearch.unit.gib": "GiB",
  "indexerSearch.unit.days": "dní",
  "indexerSearch.unit.perIndexer": "na indexer",
  "indexerSearch.saveSearch": "Uloženo",
  "indexerSearch.saveCurrentSearch": "Uložit aktuální vyhledávání",
  "indexerSearch.savedSearchesEmpty": "Zatím žádná uložená vyhledávání.",
  "indexerSearch.health.releases": "{{count}} vydání",
  "indexerSearch.health.indexers": "Indexery: {{answered}}/{{total}}",
  "indexerSearch.health.elapsed": "{{seconds}} s",
  "indexerSearch.health.stillSearching": "Stále hledá: {{count}} z {{total}}",
  "indexerSearch.health.failed": "selhalo",
  "indexerSearch.health.coolingDown": "čeká na obnovu",
  "indexerSearch.health.partial": "{{count}} · částečně",
  "indexerSearch.health.skipped": "přeskočeno",
  "indexerSearch.health.retryFailed": "Zkusit znovu",
  "indexerSearch.skipReason.indexerDisabled": "Indexer je vypnutý",
  "indexerSearch.skipReason.temporarilyDisabled": "Dočasně vypnutý",
  "indexerSearch.skipReason.temporarilyDisabledUntil":
    "Dočasně vypnutý do {{time}}",
  "indexerSearch.skipReason.backedOff": "V ústupu po nedávných selháních",
  "indexerSearch.skipReason.backedOffUntil": "V ústupu do {{time}}",
  "indexerSearch.skipReason.noTextSearch": "Nepodporuje textové vyhledávání",
  "indexerSearch.refine.title": "Upřesnit",
  "indexerSearch.refine.reset": "Obnovit výchozí",
  "indexerSearch.facet.protocol": "Protokol",
  "indexerSearch.facet.resolution": "Rozlišení",
  "indexerSearch.facet.source": "Zdroj",
  "indexerSearch.facet.audio": "Zvuk a HDR",
  "indexerSearch.facet.flags": "Příznaky",
  "indexerSearch.facet.indexer": "Indexer",
  "indexerSearch.facet.size": "Velikost",
  "indexerSearch.facet.usenet": "Usenet",
  "indexerSearch.facet.torrent": "Torrent",
  "indexerSearch.facet.atmos": "Atmos",
  "indexerSearch.facet.dolbyVision": "Dolby Vision",
  "indexerSearch.facet.hdr": "HDR",
  "indexerSearch.facet.freeleech": "Freeleech",
  "indexerSearch.facet.proper": "Proper",
  "indexerSearch.sizeRange": "{{min}} – {{max}} GiB",
  "indexerSearch.results.matched": "Zobrazeno: {{count}}",
  "indexerSearch.sort.label": "Řazení",
  "indexerSearch.column.release": "Vydání",
  "indexerSearch.column.indexer": "Indexer",
  "indexerSearch.column.size": "Velikost",
  "indexerSearch.column.age": "Stáří",
  "indexerSearch.column.peers": "Peeři / Stažení",
  "indexerSearch.row.select": "Vybrat vydání",
  "indexerSearch.row.grab": "Stáhnout",
  "indexerSearch.row.download": "Stáhnout soubor",
  "indexerSearch.download.unavailable":
    "Toto vydání je magnet odkaz a nemá soubor ke stažení",
  "indexerSearch.row.expand": "Zobrazit podrobnosti",
  "indexerSearch.row.collapse": "Skrýt podrobnosti",
  "indexerSearch.row.grabs": "{{count}} stažení",
  "indexerSearch.protocol.nzb": "NZB",
  "indexerSearch.protocol.torrent": "Torrent",
  "indexerSearch.protocol.http": "HTTP",
  "indexerSearch.detail.title": "Podrobnosti vydání",
  "indexerSearch.detail.publishDate": "Zveřejněno",
  "indexerSearch.detail.releaseGroup": "Release group",
  "indexerSearch.detail.openOnIndexer": "Otevřít na indexeru",
  "indexerSearch.detail.notAvailable": "Neuvedeno",
  "indexerSearch.age.hours": "{{count}} h",
  "indexerSearch.age.days": "{{count}} d",
  "indexerSearch.age.months": "{{count}} měs.",
  "indexerSearch.age.years": "{{count}} r.",
  "indexerSearch.footer.selected": "Vybráno: {{count}}",
  "indexerSearch.footer.nothingQueued":
    "Dokud nic nestáhnete, nic se nezařadí do fronty.",
  "indexerSearch.footer.grabSelected": "Stáhnout vybrané",
  "indexerSearch.footer.downloadSelected": "Stáhnout {{count}}",
  "indexerSearch.empty.prompt": "Zadejte dotaz a prohledejte své indexery.",
  "indexerSearch.empty.searching": "Prohledávají se indexery…",
  "indexerSearch.empty.noResults": "Žádné vydání neodpovídá.",
  "grabDialog.title.one": "Stáhnout vydání",
  "grabDialog.title.many": "Stáhnout {{count}} vydání",
  "grabDialog.subtitle":
    "Odeslat vydání do klienta pro stahování, volitelně s přiřazením k titulu v knihovně.",
  "grabDialog.summary.mix": "MIX",
  "grabDialog.summary.mixed": "Vybraná vydání: {{count}}",
  "grabDialog.assign.label": "Přiřadit k titulu",
  "grabDialog.assign.placeholder": "Hledat ve vašich titulech v knihovně…",
  "grabDialog.assign.count": "{{shown}} z {{total}} titulů",
  "grabDialog.assign.loading": "Hledají se tituly…",
  "grabDialog.assign.empty": "Žádný titul v knihovně neodpovídá.",
  "grabDialog.assign.unlinked":
    "Stáhnout bez propojení — přímo do klienta, přiřazení později v Importu",
  "grabDialog.gap.missing": "chybí {{count}}",
  "grabDialog.gap.complete": "Kompletní",
  "grabDialog.gap.wanted": "Požadované",
  "grabDialog.season": "Řada",
  "grabDialog.episode": "Epizoda",
  "grabDialog.episodic.help":
    "Nechte obě pole prázdná a Scryer rozpozná řadu a epizodu z názvu vydání.",
  "grabDialog.episodic.incomplete":
    "Zadejte řadu i epizodu, nebo nechte obě pole prázdná.",
  "grabDialog.assign.suggested": "Navrhované tituly",
  "grabDialog.routing.loading": "Zjišťují se klienti pro stahování…",
  "grabDialog.routing.single": "1 vydání",
  "grabDialog.routing.group": "{{count}} vydání",
  "grabDialog.category": "Kategorie",
  "grabDialog.category.default": "Výchozí u klienta",
  "grabDialog.footer.partial": "Odesláno: {{count}}",
  "grabDialog.cta.grab": "Stáhnout",
  "grabDialog.client": "Klient pro stahování",
  "grabDialog.client.routed": "Podle mapování indexeru",
  "grabDialog.client.placeholder": "Vyberte klienta pro stahování",
  "grabDialog.client.none": "Není zapnutý žádný klient pro stahování.",
  "grabDialog.option.replace": "Nahradit existující soubor tímto vydáním",
  "grabDialog.option.acknowledge":
    "Beru na vědomí, že toto vydání je zamítnuto podle {{codes}}",
  "grabDialog.footer.linked": "Vybraný titul: {{name}}",
  "grabDialog.footer.unlinked":
    "Nic se nepřiřadí — přiřaďte to později v Importu",
  "grabDialog.footer.pickTitle":
    "Vyberte titul z knihovny, nebo stáhněte bez propojení.",
  "grabDialog.cta.assign": "Stáhnout a přiřadit",
  "grabDialog.cta.assignAll": "Stáhnout a přiřadit vše",
  "grabDialog.cta.unlinked": "Stáhnout bez propojení",
  "grabDialog.conflict": "{{name}} už má stahování v průběhu.",
  "grabDialog.status.unlinked": "{{name}} odesláno do {{client}}",
  "grabDialog.status.failed": "{{name}}: {{reason}}",
  "grabDialog.error.expired":
    "Toto vydání už není v aktivním vyhledávání. Spusťte vyhledávání znovu.",
  "grabDialog.error.additionalScope":
    "{{name}} pokrývá více než jednu epizodu, takže ho nelze zařadit jako dodatečný soubor. Lze tak zařadit jen film nebo jednu epizodu.",
  "settings.titleTags": "Štítky",
  "settings.titleTagsExisting": "Definované štítky",
  "settings.titleTagsHelp":
    "Štítky definované zde jsou jediné, které lze na titul uplatnit. Přejmenování přepíše každý titul a profil zpoždění, který ho nese; smazání ho odstraní ze všech titulů.",
  "settings.titleTagsNone": "Zatím nejsou definovány žádné štítky.",
  "settings.titleTagsLoadError": "Štítky se nepodařilo načíst.",
  "settings.titleTagLabelLabel": "Popisek",
  "settings.titleTagLabelPlaceholder": "keep",
  "settings.titleTagLabelHelp":
    "Ukládá se malými písmeny s odstraněnými nadbytečnými mezerami. Maximálně 64 znaků.",
  "settings.titleTagDescriptionLabel": "Popis",
  "settings.titleTagDescriptionPlaceholder":
    "K čemu tento štítek slouží (volitelné)",
  "settings.titleTagTitleCount": "Titulů",
  "settings.titleTagCreate": "Definovat štítek",
  "settings.titleTagCreateNew": "Definovat nový štítek",
  "settings.titleTagEdit": "Upravit štítek",
  "settings.titleTagSaved": "Štítek uložen.",
  "settings.titleTagSaveError": "Štítek se nepodařilo uložit.",
  "settings.titleTagDeleteError": "Štítek se nepodařilo smazat.",
  "settings.titleTagDeleted":
    "Štítek {{label}} byl odebrán. Dotčených titulů: {{count}}.",
  "settings.titleTagDeleteConfirmTitle": "Smazat štítek",
  "settings.titleTagDeleteConfirm":
    "Titulů se štítkem {{label}}: {{count}}. Smazáním se odebere ze všech z nich i z každého profilu zpoždění, který na něj filtruje. Pravidla, která ho jmenují, přestanou odpovídat.",
  "settings.titleTagLabelRequired": "Zadejte popisek štítku.",
  "settings.titleTagLabelReserved":
    "Popisky nemohou začínat na scryer:, což je vyhrazeno pro nastavení, která si zapisuje sám Scryer.",
  "settings.titleTagLabelTooLong": "Popisek může mít nejvýše 64 znaků.",
  "settings.titleTagLabelInvalid": "Popisek nesmí obsahovat řídicí znaky.",
  "settings.titleTagRenameSummary":
    "Přejmenováno na {{label}}. Přepsáno – titulů: {{titles}}, seriálových filmů: {{seriesMovies}}, profilů zpoždění: {{delayProfiles}}.",
  "settings.titleTagRenameWarning":
    "Zdroje pravidel se nikdy nepřepisují. Odkazy v pravidlech, které stále jmenují starý popisek {{label}} a přestanou odpovídat ({{count}}): {{references}}.",
  "settings.titleTagReferenceMaintenanceRuleSets":
    "sady pravidel údržby: {{count}}",
  "settings.titleTagReferenceReleaseRuleSets":
    "sady pravidel hodnocení vydání: {{count}}",
  "settings.titleTagReferenceManagedTagFilters":
    "spravované filtry podle štítků: {{count}}",
  "settings.titleTagReferenceRequestRuleSets":
    "sady pravidel žádostí: {{count}}",
  "title.tagsLabel": "Štítky",
  "title.tagsNone": "Bez štítků",
  "title.tagsAdd": "Přidat štítek…",
  "title.tagsAllApplied": "Všechny definované štítky jsou už uplatněny",
  "title.tagsRemove": "Odebrat {{label}}",
  "title.tagsEmptyRegistry":
    "Zatím nejsou definovány žádné štítky. Definuje je administrátor v Nastavení > Štítky.",
  "title.tagsNoneDefined": "Zatím nejsou definovány žádné štítky",
  "title.tagsCreateMore": "Vytvořit další štítky…",
  "title.bulkAddTags": "Přidat štítky",
  "title.bulkRemoveTags": "Odebrat štítky",
  "title.catalogFilters.themes": "Témata",
  "title.catalogFilters.selectThemes": "Vybrat témata...",
  "title.catalogFilters.userTags": "Štítky",
  "title.catalogFilters.selectUserTags": "Vybrat štítky...",
  "title.catalogFilters.files": "Soubory",
  "title.catalogFilters.incompleteFiles": "Chybějící nebo částečné",
  "title.catalogFilters.onDiskFiles": "Kompletní nebo vyžadující pozornost",
  "title.catalogFilters.presenceMissing": "Chybí",
  "title.catalogFilters.presenceMissingHint": "Na disku vůbec nic není.",
  "title.catalogFilters.presencePartial": "Částečné",
  "title.catalogFilters.presencePartialHint":
    "Na disku jsou nějaké soubory a zároveň existují sledované epizody, které už byly odvysílány, ale soubor nemají. Filmy mají jediný soubor, takže nikdy nejsou částečné.",
  "title.catalogFilters.presenceComplete": "Kompletní",
  "title.catalogFilters.presenceCompleteHint":
    "Vše sledované je na disku: každá sledovaná epizoda, která byla odvysílána, má soubor, nebo je přítomen vlastní soubor filmu.",
  "title.catalogFilters.needsAttention": "Vyžaduje pozornost",
  "title.catalogFilters.needsAttentionHint":
    "Obsahuje soubor, jehož skenování selhalo nebo vyžaduje revizi, takže je nutné se na něj podívat, než ho bude možné použít.",
  "settings.titleTagSeriesMovieCount": "Seriálové filmy",
  "settings.maintenanceActionAddTags": "Přidat štítky",
  "settings.maintenanceActionRemoveTags": "Odebrat štítky",
  "settings.maintenanceRuleTags": "Štítky",
  "settings.maintenanceRuleTagsNone": "Zatím nevybrány žádné štítky",
  "settings.maintenanceRuleTagsHelp":
    "Pravidlo zapíše tyto štítky na každý titul, kterému odpovídá. Vybrat lze jen štítky, které administrátor definoval v Nastavení > Štítky.",
  "settings.maintenanceCandidatesTagPatchAdd": "přidá {{tags}}",
  "settings.maintenanceCandidatesTagPatchRemove": "odebere {{tags}}",
  "settings.maintenanceTemplateTaggedForRemovalTitle": "Označeno k odstranění",
  "settings.maintenanceTemplateTaggedForRemovalDescription":
    "Pozor: tato šablona smaže titul i jeho soubory. Odpovídá všemu, co nese štítek „remove“, po týdenní odkladné lhůtě, takže ruční označení titulu je celým spouštěčem a odebrání štítku v rámci týdne odstranění zruší. Nejprve definujte štítek „remove“ v Nastavení > Štítky.",
  "settings.maintenanceTemplateFlagForReviewTitle": "Označit k revizi",
  "settings.maintenanceTemplateFlagForReviewDescription":
    "Přidá štítek needs-review ke všemu, co bylo importováno před více než rokem, aby se mohl někdo rozhodnout, co s tím udělat. Nic se nemaže.",
  "mediaFile.allInfo": "Informace",
  "mediaFile.noAnalysis":
    "Pro tento soubor zatím není zaznamenána žádná nativní analýza.",
  "mediaInfo.sectionFile": "Soubor",
  "mediaInfo.sectionVideo": "Video",
  "mediaInfo.sectionAudio": "Zvukové stopy",
  "mediaInfo.sectionSubtitles": "Titulkové stopy",
  "mediaInfo.sectionCaptions": "Skryté titulky",
  "mediaInfo.sectionRelease": "Vydání",
  "mediaInfo.sectionChapters": "Kapitoly",
  "mediaInfo.sectionAttachments": "Přílohy",
  "mediaInfo.path": "Cesta",
  "mediaInfo.size": "Velikost",
  "mediaInfo.container": "Kontejner",
  "mediaInfo.duration": "Délka",
  "mediaInfo.chapterCount": "Kapitoly",
  "mediaInfo.scanStatus": "Stav skenování",
  "mediaInfo.role": "Role",
  "mediaInfo.added": "Přidáno",
  "mediaInfo.originalPath": "Původní cesta",
  "mediaInfo.releaseHash": "Hash vydání",
  "mediaInfo.resolution": "Rozlišení",
  "mediaInfo.videoCodec": "Kodek",
  "mediaInfo.profile": "Profil",
  "mediaInfo.bitDepth": "Bitová hloubka",
  "mediaInfo.hdr": "HDR",
  "mediaInfo.threeD": "3D",
  "mediaInfo.frameRate": "Snímková frekvence",
  "mediaInfo.videoBitrate": "Datový tok videa",
  "mediaInfo.pixelFormat": "Formát pixelů",
  "mediaInfo.fieldOrder": "Pořadí půlsnímků",
  "mediaInfo.aspectRatio": "Poměr stran",
  "mediaInfo.rotation": "Otočení",
  "mediaInfo.color": "Primární barvy / přenos / matice",
  "mediaInfo.contentLight": "MaxCLL / MaxFALL obsahu",
  "mediaInfo.masteringDisplay": "Jas mastering displeje",
  "mediaInfo.dolbyVision": "Profil Dolby Vision",
  "mediaInfo.sceneName": "Scénový název",
  "mediaInfo.releaseGroup": "Release group",
  "mediaInfo.sourceType": "Zdroj",
  "mediaInfo.parsedResolution": "Rozpoznané rozlišení",
  "mediaInfo.parsedVideoCodec": "Rozpoznaný video kodek",
  "mediaInfo.parsedAudioCodec": "Rozpoznaný zvukový kodek",
  "mediaInfo.edition": "Edice",
  "mediaInfo.indexer": "Indexer",
  "mediaInfo.grabbedTitle": "Stažené vydání",
  "mediaInfo.grabbedAt": "Staženo",
  "mediaInfo.acquisitionScore": "Skóre získání",
  "mediaInfo.scoringLog": "Protokol hodnocení",
  "mediaInfo.sectionReleaseListing": "Záznam z indexeru",
  "mediaInfo.listingPublishedAt": "Zveřejněno",
  "mediaInfo.listingAgeAtGrab": "Stáří při stažení",
  "mediaInfo.listingAgeDay": "{{count}} den",
  "mediaInfo.listingAgeDays": "{{count}} d",
  "mediaInfo.listingThumbsUp": "Kladné hlasy",
  "mediaInfo.listingThumbsDown": "Záporné hlasy",
  "mediaInfo.listingPasswordProtected": "Chráněno heslem",
  "mediaInfo.listingIndexerLanguages": "Jazyky podle indexeru",
  "mediaInfo.listingCapturedAt": "Zaznamenáno",
  "mediaInfo.colIndex": "#",
  "mediaInfo.colLanguage": "Jazyk",
  "mediaInfo.colCodec": "Kodek",
  "mediaInfo.colProfile": "Profil",
  "mediaInfo.colChannels": "Kanály",
  "mediaInfo.colBitrate": "Datový tok",
  "mediaInfo.colSampleRate": "Vzorkovací frekvence",
  "mediaInfo.colSampleDepth": "Bitová hloubka vzorku",
  "mediaInfo.colRoles": "Role",
  "mediaInfo.colName": "Název",
  "mediaInfo.inferredLanguageFootnote":
    "Odvozeno z názvu stopy, protože stopa nemá pole jazyka.",
  "mediaInfo.colForced": "Vynucené",
  "mediaInfo.colDefault": "Výchozí",
  "mediaInfo.colStart": "Začátek",
  "mediaInfo.colEnd": "Konec",
  "mediaInfo.colTitle": "Název",
  "mediaInfo.colType": "Typ",
  "mediaInfo.colSize": "Velikost",
  "mediaInfo.roleDefault": "Výchozí",
  "mediaInfo.roleForced": "Vynucené",
  "mediaInfo.roleOriginal": "Originál",
  "nav.lists": "Seznamy",
  "lists.commandDescription":
    "Sledujte veřejné seznamy a žebříčky a automaticky přidávejte jejich tituly.",
  "lists.tab.public": "Veřejné seznamy",
  "lists.tab.exclusions": "Výjimky",
  "lists.exclusions.commandDescription":
    "Tituly, které žádný seznam nesmí přidat.",
  "lists.action.enabled": "Sledovat {{name}}",
  "lists.action.retry": "Zkusit znovu",
  "lists.action.syncAll": "Synchronizovat vše",
  "lists.action.syncNow": "Synchronizovat nyní",
  "lists.action.unfollow": "Přestat sledovat",
  "lists.url.label": "Sledovat seznam podle URL",
  "lists.url.placeholder": "Vložte URL seznamu nebo žebříčku",
  "lists.url.submit": "Zkontrolovat seznam",
  "lists.url.help": "Vložte adresu seznamu od podporovaného poskytovatele.",
  "lists.url.recognized": "{{provider}} · {{list}}",
  "lists.url.unrecognized":
    "Tuto adresu Scryer neumí sledovat jako seznam. Podporované seznamy najdete v katalogu poskytovatelů níže.",
  "lists.followed.heading": "Sledované seznamy",
  "lists.followed.empty": "Zatím nejsou sledovány žádné veřejné seznamy.",
  "lists.followed.emptyManager":
    "Zatím žádné veřejné seznamy. Vložte výše adresu URL seznamu nebo vyberte některý z katalogu níže.",
  "lists.table.list": "Seznam",
  "lists.table.coverage": "Pokrytí",
  "lists.table.mode": "Režim",
  "lists.table.lastSync": "Poslední synchronizace",
  "lists.table.neverSynced": "Zatím nesynchronizováno",
  "lists.catalog.heading": "Procházet seznamy",
  "lists.catalog.providers": "Poskytovatelé seznamů",
  "lists.catalog.empty": "Nejsou nainstalováni žádní poskytovatelé seznamů.",
  "lists.catalog.follow": "Sledovat",
  "lists.catalog.followed": "Sledováno",
  "lists.catalog.every": "Synchronizace každých {{interval}}",
  "lists.catalog.auth.memberAccount": "Účet člena",
  "lists.catalog.auth.serverKey": "Klíč API serveru",
  "lists.interval.minutes": "{{count}} min",
  "lists.interval.hours": "{{count}} h",
  "lists.interval.days": "{{count}} d",
  "lists.kind.movie": "Filmy",
  "lists.kind.series": "Seriály",
  "lists.kind.anime": "Anime",
  "lists.mode.add": "Přidat",
  "lists.mode.hold": "Čekat na schválení",
  "lists.mode.request": "Požádat",
  "lists.mode.discover": "Zobrazit jen v Objevování",
  "lists.modeHelp.add":
    "Přidá nové tituly se sledováním zvoleným pro daný typ. Sledované tituly se vyhledávají, nesledované ne.",
  "lists.modeHelp.hold":
    "Pro každý nový titul vytvoří čekající žádost, kterou může někdo schválit.",
  "lists.modeHelp.request": "Požádá o nové tituly jménem člena.",
  "lists.modeHelp.discover": "Zaznamená tituly ze seznamu, ale nic nepřidává.",
  "lists.mode.discoverUnavailable":
    "Dostupné, jakmile se seznamy objeví v Objevování.",
  "lists.onLeave.keep": "Ponechat titul",
  "lists.onLeave.log": "Zaznamenat do protokolu",
  "lists.onLeave.unmonitor": "Zrušit sledování titulu",
  "lists.onLeave.tag": "Označit titul štítkem",
  "lists.syncState.ok": "Synchronizováno",
  "lists.syncState.new": "Zatím nesynchronizováno",
  "lists.syncState.waiting": "Čeká na synchronizaci",
  "lists.syncState.fail": "Synchronizace selhala",
  "lists.syncState.off": "Pozastaveno",
  "lists.membershipState.inLibrary": "V knihovně",
  "lists.membershipState.added": "Přidáno",
  "lists.membershipState.requested": "Požádáno",
  "lists.membershipState.held": "Čeká na schválení",
  "lists.membershipState.rejected": "Zamítnuto",
  "lists.membershipState.filtered": "Odfiltrováno",
  "lists.membershipState.excluded": "Vyloučeno",
  "lists.membershipState.unresolved": "Nelze spárovat",
  "lists.membershipState.blockedPermission": "Nepovoleno",
  "lists.membershipState.discover": "Jen v Objevování",
  "lists.membershipState.pending": "Čeká",
  "lists.reason.mediaTypeNotIncluded":
    "Tento seznam tento typ titulu neukládá.",
  "lists.reason.noRoute": "Pro tento typ titulu není zvolena žádná knihovna.",
  "lists.reason.duplicateTarget":
    "Stejný titul už obsahuje jiná položka tohoto seznamu.",
  "lists.reason.specials": "Speciály se z tohoto seznamu neukládají.",
  "lists.reason.filler": "Vynecháno nastavením výplňových dílů.",
  "lists.reason.recap": "Vynecháno nastavením souhrnných dílů.",
  "lists.reason.rating": "Jeho hodnocení je nižší než minimum.",
  "lists.reason.missingRating":
    "Nemá hodnocení, se kterým by šlo porovnat minimum.",
  "lists.reason.genre": "Má žánr nebo téma, které tento seznam vynechává.",
  "lists.reason.missingGenre":
    "Jeho žánry zatím nejsou známé, takže ho filtr žánrů nemůže propustit.",
  "lists.reason.adult":
    "Jde o titul pro dospělé, který tento seznam vynechává.",
  "lists.reason.missingAdult":
    "Zatím není známo, zda jde o titul pro dospělé, takže ho tento filtr nemůže propustit.",
  "lists.reason.releaseYear": "Rok vydání je mimo zvolený rozsah.",
  "lists.reason.missingReleaseYear": "Rok vydání není známý.",
  "lists.reason.format":
    "Jeho formát není mezi těmi, které tento seznam ukládá.",
  "lists.reason.language":
    "Jeho původní jazyk není mezi těmi, které tento seznam ukládá.",
  "lists.reason.missingLanguage": "Jeho původní jazyk není známý.",
  "lists.reason.streamingService":
    "Je dostupný na některé z vašich streamovacích služeb.",
  "lists.reason.unreleased": "Zatím nebyl vydán.",
  "lists.reason.missingReleaseDate": "Datum vydání není známé.",
  "lists.reason.directorCredit": "Nejde o režijní podíl.",
  "lists.reason.sequelWithoutBase":
    "Jde o pokračování titulu, který není v knihovně.",
  "lists.reason.ambiguousSeriesMovie":
    "Mohlo by jít o více než jeden film ze série, proto nebyl spárován.",
  "lists.reason.notPermitted":
    "Vlastník seznamu nemá povoleno titul přidat nebo o něj požádat.",
  "lists.reason.refused": "Přidání bylo odmítnuto.",
  "lists.reason.notFound": "Při přidávání se titul nepodařilo najít.",
  "lists.reason.actionFailed":
    "Přidání se nezdařilo. Příští synchronizace to zkusí znovu.",
  "lists.reason.requestRejected": "Žádost o tento titul byla zamítnuta.",
  "lists.runOutcome.succeeded": "Úspěšné",
  "lists.runOutcome.failed": "Selhalo",
  "lists.runOutcome.skipped": "Přeskočeno",
  "lists.counts.inLibrary": "V knihovně",
  "lists.counts.added": "Přidáno",
  "lists.counts.requested": "Požádáno",
  "lists.counts.held": "Čeká na schválení",
  "lists.counts.filtered": "Odfiltrováno",
  "lists.counts.excluded": "Vyloučeno",
  "lists.counts.unresolved": "Nelze spárovat",
  "lists.counts.empty": "Zatím nic nesynchronizováno",
  "lists.follow.title": "Sledovat seznam",
  "lists.follow.editTitle": "Upravit seznam",
  "lists.follow.submit": "Sledovat seznam",
  "lists.follow.sourceHeading": "Podrobnosti seznamu",
  "lists.follow.order": "Pořadí seznamu",
  "lists.follow.orderHint": "Tituly se ze seznamu čtou v tomto pořadí.",
  "lists.follow.orderUnset": "Výchozí pořadí",
  "lists.follow.order.rank": "Vlastní pořadí seznamu",
  "lists.follow.order.added": "Datum přidání",
  "lists.follow.order.released": "Datum vydání",
  "lists.follow.name": "Název",
  "lists.follow.kinds": "Zahrnout",
  "lists.follow.modeHeading": "Když se objeví nový titul",
  "lists.follow.routesHeading": "Kam se tituly ukládají",
  "lists.follow.routesHelp":
    "Každý typ titulu potřebuje knihovnu. Typy bez knihovny se nikdy nepřidávají.",
  "lists.follow.filtersHeading": "Filtry",
  "lists.follow.syncHeading": "Synchronizace",
  "lists.follow.maxPerSync": "Nejvíce titulů na synchronizaci",
  "lists.follow.maxPerSyncPlaceholder": "Bez limitu",
  "lists.follow.maxPerSyncHelp":
    "Tituly nad limit čekají na další synchronizaci.",
  "lists.follow.onLeave": "Když titul opustí seznam",
  "lists.follow.onLeaveHelp":
    "Platí jen pro tituly, které přidal tento seznam a nedrží je žádný jiný seznam. Seznamy nikdy tituly nemažou.",
  "lists.follow.interval": "Rozvrh určuje poskytovatel: každých {{interval}}.",
  "lists.follow.problem.name": "Zadejte název seznamu.",
  "lists.follow.problem.kinds": "Zahrňte alespoň jeden typ titulu.",
  "lists.follow.problem.mode": "Zvolte, co se stane s novými tituly.",
  "lists.follow.problem.route": "Zvolte knihovnu pro každý zahrnutý typ.",
  "lists.follow.problem.maxPerSync": "Limit musí být alespoň 1.",
  "lists.follow.problem.params": "Vyplňte požadované údaje seznamu.",
  "lists.route.heading": "{{kind}}",
  "lists.route.noLibrary": "Pro tento typ titulu nespravujete žádnou knihovnu.",
  "lists.filter.rating": "Minimální hodnocení",
  "lists.filter.ratings": "Hodnocení",
  "lists.filter.all": "Všechna",
  "lists.filter.any": "Alespoň jedno",
  "lists.filter.match": "Shoda s minimy hodnocení",
  "lists.filter.canonical": "Vyloučené žánry/témata",
  "lists.filter.searchCanonical": "Hledat žánry a témata…",
  "lists.filter.vocabularyError": "Kanonické žánry a témata nejsou dostupné.",
  "lists.filter.retry": "Zkusit znovu",
  "lists.filter.unavailable": "Nedostupné: {{label}}",
  "lists.filter.unresolved": "Vyžaduje opravu: {{label}}",
  "lists.filter.excludeAdult": "Vyloučit tituly pro dospělé",
  "lists.filter.excludeAdultHelp":
    "Vynechá tituly označené jako pro dospělé. Titul, jehož údaje zatím nejsou známé, je vynechán, dokud nebudou.",
  "title.tagsCreateSelect": "Vytvořit „{{label}}“ a vybrat",
  "title.tagsSearch": "Hledat nebo vytvořit štítek…",
  "lists.filter.ratingScale": "Zdroj hodnocení",
  "lists.filter.ratingPlaceholder": "Libovolný",
  "lists.filter.scale.tmdb": "TMDB",
  "lists.filter.scale.imdb": "IMDb",
  "lists.filter.releaseYear": "Rok vydání",
  "lists.filter.yearFrom": "Od",
  "lists.filter.yearTo": "Do",
  "lists.filter.excludeGenres": "Přeskočit žánry",
  "lists.filter.excludeGenresPlaceholder": "horor, dokument",
  "lists.filter.languages": "Původní jazyky",
  "lists.filter.languagesPlaceholder": "en, ja",
  "lists.filter.releasedOnly": "Jen vydané tituly",
  "lists.filter.releasedOnlyHelp":
    "Přeskočí tituly, které ještě nebyly vydány. Při každé synchronizaci se zkontrolují znovu.",
  "lists.preview.heading": "Náhled",
  "lists.preview.nextSyncHeading": "Příští synchronizace by přidala",
  "lists.preview.run": "Náhled",
  "lists.preview.notRun":
    "Zobrazte náhled seznamu a uvidíte, co by přidala první synchronizace.",
  "lists.preview.failed": "Náhled tohoto seznamu se nezdařil.",
  "lists.preview.total": "Na seznamu",
  "lists.preview.inLibrary": "V knihovně",
  "lists.preview.wouldAdd": "Přidalo by se",
  "lists.preview.filtered": "Odfiltrováno",
  "lists.preview.excluded": "Vyloučeno",
  "lists.preview.unresolved": "Bez shody",
  "lists.preview.unresolvedHelp":
    "Dosud nespárovaných titulů: {{count}}. Při každé synchronizaci se zkusí znovu.",
  "lists.preview.wouldAddHeading":
    "Tituly, které by přidala příští synchronizace",
  "lists.preview.nothingToAdd": "Momentálně není co nového přidat.",
  "lists.detail.coverage": "Pokrytí",
  "lists.detail.settings": "Nastavení",
  "lists.detail.interval": "Rozvrh",
  "lists.detail.nextSync": "Příští synchronizace",
  "lists.detail.noCap": "Bez limitu",
  "lists.detail.openAtProvider": "Otevřít u poskytovatele",
  "lists.detail.pausedUntil":
    "Poskytovatel požádal Scryer o zpomalení. Synchronizace bude pokračovat po {{time}}.",
  "lists.detail.titles": "Tituly ({{count}})",
  "lists.detail.title": "Titul",
  "lists.detail.state": "Stav",
  "lists.detail.noTitles": "Zatím nejsou zaznamenány žádné tituly.",
  "lists.detail.sourceRank": "Pořadí u zdroje",
  "lists.detail.pageRange": "Položky {{from}}–{{to}} z {{total}}",
  "lists.detail.previousPage": "Předchozí",
  "lists.detail.nextPage": "Další",
  "lists.detail.history": "Historie synchronizací",
  "lists.detail.noRuns": "Zatím žádné synchronizace.",
  "lists.detail.olderRuns": "Zobrazit starší synchronizace",
  "lists.detail.runCounts":
    "Na seznamu: {{total}} · přidáno: {{added}} · čeká na schválení: {{held}}",
  "lists.unfollow.title": "Přestat sledovat {{name}}?",
  "lists.unfollow.description":
    "Scryer přestane tento seznam synchronizovat. Tituly zůstanou ve vaší knihovně a čekající žádosti zůstanou v Žádostech.",
  "lists.status.followed": "Sledování {{name}} zahájeno.",
  "lists.status.updated": "Uloženo: {{name}}.",
  "lists.status.unfollowed": "Sledování {{name}} ukončeno.",
  "lists.status.syncQueued": "Synchronizace {{name}} zahájena.",
  "lists.status.syncAllQueued": "Synchronizace všech seznamů zahájena.",
  "lists.refusal.alreadyFollowed": "Tento seznam je už sledován.",
  "lists.refusal.sourceRequired":
    "Zvolte poskytovatele a seznam, nebo vložte odkaz.",
  "lists.refusal.linkNotRecognized":
    "Tento odkaz není seznam, který Scryer umí sledovat.",
  "lists.refusal.providerNotInstalled":
    "Tento poskytovatel seznamů není nainstalován.",
  "lists.refusal.providerUnavailable":
    "Tento poskytovatel seznamů je momentálně nedostupný.",
  "lists.refusal.sourceNotOffered": "Tento poskytovatel tento seznam nenabízí.",
  "lists.refusal.chartUnavailable": "Tento žebříček už není dostupný.",
  "lists.refusal.memberAccountOnly":
    "Tento seznam vyžaduje vlastní účet člena a nelze ho sledovat pro všechny.",
  "lists.refusal.modeNotAllowed":
    "Tato akce není pro tento typ seznamu dostupná.",
  "lists.refusal.kindsRequired": "Zvolte alespoň jeden typ titulu.",
  "lists.refusal.routeLibraryRequired": "Zvolte knihovnu pro každý typ titulu.",
  "lists.refusal.routeLibraryNotFound": "Zvolená knihovna už neexistuje.",
  "lists.refusal.syncCapInvalid":
    "Nejvyšší počet titulů na synchronizaci musí být alespoň 1.",
  "lists.refusal.disabled":
    "Tento seznam je vypnutý. Zapněte ho, aby se synchronizoval.",
  "lists.refusal.exclusionIdsRequired": "Výjimka potřebuje alespoň jedno ID.",
  "lists.refusal.exclusionTitleRequired": "Výjimka potřebuje titul.",
  "lists.refusal.accountRequired": "Zvolte připojený účet.",
  "lists.refusal.accountReconnectRequired":
    "Pro další používání tento účet znovu připojte.",
  "lists.refusal.accountProviderMismatch":
    "Tento účet patří jinému poskytovateli.",
  "lists.refusal.accountIdentityMissing":
    "Poskytovatel neuvedl, čí je to účet. Připojte ho znovu.",
  "lists.refusal.accountIdentityUnverified":
    "Účet se nepodařilo ověřit. Připojte ho znovu.",
  "lists.refusal.accountLinkingUnsupported":
    "Tento poskytovatel nepodporuje připojení účtu.",
  "lists.refusal.accountLinkOriginInvalid":
    "Propojení účtu nelze z této adresy zahájit.",
  "lists.refusal.accountLinksTooMany":
    "Čeká na dokončení příliš mnoho propojení účtů. Některé dokončete nebo počkejte, až vyprší, a pak se připojte znovu.",
  "lists.refusal.accountLinkResultInvalid":
    "Odpověď poskytovatele nebyla použitelná. Zahajte připojení znovu.",
  "lists.refusal.accountLinkIssuerInvalid":
    "Odpověď nepřišla od poskytovatele. Zahajte připojení znovu.",
  "lists.refusal.accountKeyUnsupported":
    "Účty tohoto poskytovatele se nepřipojují klíčem.",
  "lists.refusal.accountKeyRequired":
    "Tento poskytovatel se připojuje vlastním klíčem, ale nainstalovaný plugin poskytovatele o něj nežádá. Administrátor musí plugin aktualizovat.",
  "lists.refusal.accountKeyInvalid":
    "To nevypadá jako klíč. Vložte ho jako jeden řádek bez mezer.",
  "lists.refusal.accountKeyRejected":
    "Poskytovatel tento klíč nepřijal. Zkontrolujte, že jste zkopírovali celý klíč, a zkuste to znovu.",
  "lists.refusal.accountAuthUnavailable":
    "Připojení účtů nejsou na tomto serveru dostupná.",
  "lists.refusal.accountAuthAccessDenied":
    "Poskytovatel neudělil přístup. Připojte se znovu a žádost schvalte.",
  "lists.refusal.accountAuthAuthorizationFailed":
    "Poskytovatel schválení nepřijal. Zahajte připojení znovu.",
  "lists.refusal.accountAuthBusy":
    "Scryer momentálně zpracovává příliš mnoho připojení účtů. Zkuste to za chvíli znovu.",
  "lists.refusal.accountAuthInstanceUnrecognized":
    "Přihlašovací služba Scryeru nemohla tento server potvrdit. Zkuste to za pár minut znovu. Pokud to přetrvává, nechte administrátora zkontrolovat, že hodiny serveru jdou správně a že má přístup k internetu.",
  "lists.refusal.accountAuthInvalidProviderApp":
    "Vlastní aplikace tohoto serveru pro poskytovatele má zpětnou adresu, která neodpovídá adrese, na níž je Scryer otevřený. Administrátor to musí opravit v nastavení poskytovatele.",
  "lists.refusal.accountAuthInvalidProviderResponse":
    "Odpověď poskytovatele nebyla použitelná. Zahajte připojení znovu.",
  "lists.refusal.accountAuthInvalidRequest":
    "Žádost o připojení nebyla použitelná. Zahajte připojení znovu.",
  "lists.refusal.accountAuthProviderNotConfigured":
    "Tento poskytovatel zatím není nastaven pro připojování účtů. Administrátor musí nastavení poskytovatele dokončit.",
  "lists.refusal.accountAuthProviderRejected":
    "Poskytovatel připojení odmítl. Zahajte připojení znovu.",
  "lists.refusal.accountAuthProviderUnavailable":
    "Poskytovatel momentálně neodpovídá. Zkuste to později.",
  "lists.refusal.accountAuthRateLimited":
    "Poskytovatel požádal Scryer o zpomalení. Počkejte minutu a zkuste to znovu.",
  "lists.refusal.accountAuthReconnectRequired":
    "Poskytovatel toto přihlášení už nepřijímá. Zahajte připojení znovu.",
  "lists.refusal.accountAuthRelayUnavailable":
    "Přihlašovací služba Scryeru momentálně neodpovídá. Zkuste to později.",
  "lists.refusal.accountAuthTransportUnavailable":
    "Tento server se nemohl spojit s poskytovatelem. Zkontrolujte jeho připojení k internetu a zkuste to znovu.",
  "lists.refusal.accountAuthUnsupported":
    "Účty tohoto poskytovatele nelze tímto způsobem připojit.",
  "lists.refusal.accountAuthUnsupportedProviderApp":
    "Tento poskytovatel nepřijímá vlastní aplikaci. Administrátor ji musí z nastavení poskytovatele odstranit.",
  "lists.refusal.providerAppUnsupported":
    "Tento poskytovatel nepřijímá vlastní aplikaci.",
  "lists.refusal.providerAppRedirectInvalid":
    "Přesměrovací URI není platná adresa URL.",
  "lists.refusal.providerAppIncomplete":
    "Aplikace poskytovatele potřebuje ID klienta, tajný klíč tam, kde ho poskytovatel vyžaduje, a platné zpětné URI pro účet.",
  "lists.exclusions.addHeading": "Vyloučit titul",
  "lists.exclusions.titleLabel": "Titul",
  "lists.exclusions.titlePlaceholder": "Vyhledat titul",
  "lists.exclusions.yearLabel": "Rok",
  "lists.exclusions.kindLabel": "Typ",
  "lists.exclusions.idsLabel": "Externí ID",
  "lists.exclusions.idsHelp": "Oddělte čárkami ve formátu zdroj:id.",
  "lists.exclusions.scopeLabel": "Platí pro",
  "lists.exclusions.scopeAll": "Všechny seznamy",
  "lists.exclusions.scopeList": "Jen {{name}}",
  "lists.exclusions.add": "Vyloučit",
  "lists.exclusions.added": "{{name}} je ze seznamů vyloučen.",
  "lists.exclusions.removed": "{{name}} mohou seznamy opět přidávat.",
  "lists.exclusions.empty": "Žádné tituly nejsou vyloučeny.",
  "lists.exclusions.created": "Přidáno",
  "lists.exclusions.remove": "Odebrat výjimku pro {{name}}",
  "lists.exclusions.removeTitle": "Odebrat výjimku pro {{name}}?",
  "lists.exclusions.removeDescription":
    "Seznamy mohou tento titul při příští synchronizaci přidat znovu.",
  "lists.exclusions.deleteCheckbox": "Vyloučit také ze seznamů",
  "lists.exclusions.deleteNoIds":
    "{{name}} byl odebrán, ale nemá žádná externí ID, podle kterých by se dal vyloučit ze seznamů.",
  "lists.exclusions.deleteFailed":
    "{{name}} byl odebrán, ale vyloučení ze seznamů se nezdařilo. Přidejte výjimku na stránce Seznamy.",
  "lists.exclusions.bulkDeleteNoIds":
    "Odebraných titulů bez externích ID pro vyloučení ze seznamů: {{count}}.",
  "lists.exclusions.bulkDeleteFailed":
    "Odebrané tituly ({{count}}) se nepodařilo vyloučit ze seznamů. Přidejte tyto výjimky na stránce Seznamy.",
  "lists.policy.heading": "Zásada seznamů členů",
  "lists.policy.help": "Určuje, jak se osobní seznamy členů mění na žádosti.",
  "lists.policy.empty": "Žádní členové k zobrazení.",
  "lists.policy.member": "Člen",
  "lists.policy.requests30d": "Žádosti ze seznamů (30 dní)",
  "lists.policy.policy": "Zásada",
  "lists.policy.policyFor": "Zásada seznamů pro {{name}}",
  "lists.policy.value.auto": "Automaticky schválit",
  "lists.policy.value.approval": "Vyžaduje schválení",
  "lists.policy.value.none": "Seznamy nepovoleny",
  "lists.providerSettings.heading": "Nastavení poskytovatele",
  "lists.providerSettings.copy":
    "Sdíleno všemi seznamy od tohoto poskytovatele na tomto serveru.",
  "lists.providerSettings.required": "Povinné",
  "lists.providerSettings.secretStored": "Uloženo",
  "lists.providerSettings.secretEmpty": "Nenastaveno",
  "lists.providerSettings.secretPlaceholder":
    "Ponechte prázdné pro zachování uložené hodnoty",
  "lists.providerSettings.clearSecret": "Vymazat",
  "lists.providerSettings.keepSecret": "Ponechat",
  "lists.providerSettings.save": "Uložit nastavení",
  "lists.providerSettings.saved": "Nastavení poskytovatele uloženo.",
  "lists.tab.personal": "Osobní",
  "lists.accounts.heading": "Vaše účty",
  "lists.accounts.copy":
    "Připojte účet, abyste mohli sledovat své soukromé seznamy a sledovaná videa. Vaše účty a osobní seznamy vidíte jen vy.",
  "lists.accounts.connect": "Připojit",
  "lists.accounts.installProviders":
    "Nejprve níže nainstalujte a zapněte poskytovatele osobních seznamů, pak zde připojte svůj účet.",
  "lists.plugins.heading": "Pluginy poskytovatelů seznamů",
  "lists.accounts.askForProviders":
    "Požádejte administrátora, aby nainstaloval a zapnul plugin poskytovatele seznamů, abyste mohli připojit svůj účet a sledovat soukromé seznamy nebo sledovaná videa.",
  "profile.listAccounts": "Účty osobních seznamů",
  "profile.mediaServerAccounts": "Účty mediálního serveru",
  "lists.accounts.connected": "Připojeno",
  "lists.accounts.reconnect": "Připojit znovu",
  "lists.accounts.connectAnother": "Připojit další účet",
  "lists.accounts.connectAnotherHint":
    "Poskytovatel připojí ten účet, který je u něj právě přihlášený. Chcete-li přidat jiný, přihlaste se k němu v okně poskytovatele.",
  "lists.accounts.notConnected": "Nepřipojeno",
  "lists.accounts.needsReconnect": "Je třeba znovu připojit",
  "lists.accounts.manage": "Spravovat",
  "lists.accounts.manageTitle": "Spravovat účet {{provider}}",
  "lists.accounts.private": "Tyto seznamy a údaje o účtu vidíte jen vy.",
  "lists.accounts.waiting":
    "Dokončete autorizaci v okně poskytovatele. Schválení se kontroluje automaticky.",
  "lists.accounts.key.checking": "Ověřování klíče u poskytovatele…",
  "lists.accounts.key.where": "Kde ho najít",
  "lists.accounts.key.private":
    "Váš klíč je na tomto serveru uložen zašifrovaně, používá se jen pro vaše vlastní seznamy a už se znovu nezobrazí.",
  "lists.accounts.key.replace":
    "Uložením nového klíče nahradíte ten, který je pro tento účet uložen.",
  "lists.accounts.key.approval":
    "Po přijetí klíče se otevře okno poskytovatele, kde připojení schválíte.",
  "lists.accounts.key.missing": "Nejprve zadejte svůj klíč.",
  "lists.accounts.linkFailed":
    "Propojení účtu se nezdařilo. Začněte znovu a připojte se zas.",
  "lists.accounts.pollRetrying":
    "Schválení se teď nepodařilo zkontrolovat. Zkouší se dál, dokud relace nevyprší.",
  "lists.accounts.originNotAllowed":
    "Propojení účtu z této adresy není dostupné. Pokud je Scryer dostupný přes publikovaný port kontejneru, reverzní proxy nebo hostitelský název, požádejte administrátora, aby nastavil veřejnou adresu URL Scryeru; jinak otevřete Scryer na localhost nebo adrese, na které naslouchá.",
  "lists.accounts.linkExpired":
    "Tato relace propojení účtu vypršela. Zahajte nové připojení ze Seznamů.",
  "lists.accounts.popupBlocked":
    "Povolte vyskakovací okna pro schválení tohoto účtu a připojte se znovu.",
  "lists.accounts.finishing": "Dokončování připojení účtu…",
  "lists.accounts.linked": "Účet připojen.",
  "lists.accounts.returnToLists": "Zpět na vaše osobní seznamy",
  "lists.accounts.noProviders":
    "Nejsou nainstalováni žádní poskytovatelé osobních seznamů.",
  "lists.accounts.ownedLists": "Vaše seznamy",
  "lists.accounts.statuses": "Dostupné stavy: {{statuses}}",
  "lists.accounts.unlink": "Odpojit účet",
  "lists.accounts.confirmUnlink": "Potvrdit odpojení",
  "lists.accounts.unlinkHelp":
    "Odpojením se odeberou odběry používající tento účet. Soubory ve vaší knihovně zůstanou zachovány.",
  "lists.accounts.unlinked": "Účet odpojen.",
  "lists.accounts.personalRouting":
    "Osobní seznamy patří vám. Režimy Přidat a Požádat vyžadují přístup ke správě titulů v každé zvolené knihovně. Žádosti se řídí vaší zásadou seznamů a pravidly žádostí.",
  "lists.providerApps.heading": "Vlastní OAuth",
  "lists.providerApps.settingsLabel": "Seznamy · Vlastní OAuth",
  "lists.providerApps.copy":
    "Výchozí aplikace projektu fungují bez registrace. Volitelně můžete zaregistrovat vlastní aplikaci Trakt, AniList nebo MyAnimeList pro přímé připojení z této instance.",
  "lists.providerApps.simklDefault": "Simkl používá výchozí aplikaci projektu.",
  "lists.providerApps.useOwn": "Použít vlastní aplikaci poskytovatele",
  "lists.providerApps.clientId": "ID klienta",
  "lists.providerApps.clientSecret": "Tajný klíč klienta",
  "lists.providerApps.traktSecretHint":
    "U Traktu volitelné. Trakt přihlašuje členy pomocí PKCE; tajný klíč potřebují jen starší aplikace Trakt.",
  "lists.providerApps.redirectUri": "Přesměrovací URI",
  "lists.providerApps.redirectHelp":
    "Zaregistrujte u poskytovatele adresu URL /lists/oauth/callback této instance. Ponechte tajný klíč prázdný pro zachování uložené hodnoty.",
  "title.listProvenance.added": "Přidal seznam {{name}}",
  "title.listProvenance.left": "Opustil seznam {{name}}",
  "script.editor.jobNamePlaceholder": "např. Noční úklid",
  "script.editor.inline": "Vložený kód",
  "script.editor.interpreter": "Interpret",
  "script.editor.interpreterHelp":
    "Interpret, který skript spouští. Cesty k interpretům nastavte v Nastavení > Skripty.",
  "script.editor.format": "Formátovat odsazení",
  "script.editor.formatHelp":
    "Přeodsadí skript podle zvoleného jazyka (Shift+Alt+F).",
  "script.editor.assistanceHint":
    "Ctrl+Mezerník: návrhy · Tab: odsazení · Shift+Alt+F: formátovat odsazení",
  "script.editor.injectedVariable": "Proměnná prostředí Scryeru",
  "script.editor.snippet": "Úryvek jazyka",
  "script.editor.env.SCRYER_METADATA":
    "Metadata JSON popisující tento import nebo plánovanou úlohu. Přečtěte je z prostředí a zpracujte jako JSON.",
  "script.editor.env.SCRYER_EVENT":
    "Událost, která skript spustila: post_import nebo scheduled_job.",
  "script.editor.env.SCRYER_FILE_PATH":
    "Úplná cesta k importovanému mediálnímu souboru.",
  "script.editor.env.SCRYER_FACET":
    "Mediální kategorie importovaného titulu: movie, series nebo anime.",
  "script.editor.env.SCRYER_TITLE_NAME": "Název importovaného titulu.",
  "script.editor.env.SCRYER_TITLE_ID":
    "Identifikátor importovaného titulu ve Scryeru.",
  "script.editor.env.SCRYER_JOB_ID":
    "Identifikátor plánované úlohy ve Scryeru.",
  "script.editor.env.SCRYER_JOB_NAME": "Název plánované úlohy.",
  "script.editor.env.SCRYER_RUN_ID":
    "Identifikátor tohoto běhu plánované úlohy.",
  "script.editor.env.SCRYER_TRIGGER_SOURCE":
    "Jak byl tento běh úlohy spuštěn, například manual, startup nebo schedule.",
  "script.editor.inlineHelp": "Skript se spouští v jazyce zvoleném výše.",
  "script.editor.filePathHelp":
    "Absolutní cesta ke skriptu na hostiteli Scryeru.",
  "script.editor.selectFile": "Vybrat soubor skriptu",
  "script.editor.fileRequired": "Před uložením vyberte soubor skriptu.",
  "script.editor.required": "Jsou vyžadovány název a skript.",
  "script.editor.schedule": "Plán",
  "script.editor.blockingHelp":
    "Čeká na dokončení skriptu a zaznamená jeho návratový kód a výstup.",
  "script.editor.fireAndForgetHelp":
    "Spustí skript a zaznamená jen to, že byl spuštěn.",
  "script.language.shell": "Shell",
  "script.language.python": "Python",
  "script.language.powershell": "PowerShell",
  "script.language.batch": "Batch",
  "script.language.go": "Go",
  "script.output.filter": "Výstupní proud",
  "script.output.filter.combined": "Kombinovaný",
  "script.output.filter.stdout": "Stdout",
  "script.output.filter.stderr": "Stderr",
  "script.output.stream.stdout": "stdout",
  "script.output.stream.stderr": "stderr",
  "script.schedule.kind": "Spouštění",
  "script.schedule.kind.manual": "Ručně",
  "script.schedule.kind.interval": "Interval",
  "script.schedule.kind.daily": "Denně",
  "script.schedule.kind.weekly": "Týdně",
  "script.schedule.kind.cron": "Cron",
  "script.schedule.runOnStartup": "Spustit také při startu Scryeru",
  "script.schedule.manualHelp":
    "Úloha se spustí jen tehdy, zvolíte-li Spustit nyní.",
  "script.schedule.every": "Každých",
  "script.schedule.unit": "Jednotka",
  "script.schedule.unit.minutes": "Minuty",
  "script.schedule.unit.hours": "Hodiny",
  "script.schedule.unit.days": "Dny",
  "script.schedule.minimumInterval": "Nejkratší interval je jedna minuta.",
  "script.schedule.time": "Čas",
  "script.schedule.days": "Dny",
  "script.schedule.day.monday": "Po",
  "script.schedule.day.tuesday": "Út",
  "script.schedule.day.wednesday": "St",
  "script.schedule.day.thursday": "Čt",
  "script.schedule.day.friday": "Pá",
  "script.schedule.day.saturday": "So",
  "script.schedule.day.sunday": "Ne",
  "script.schedule.cronExpression": "Výraz cron",
  "script.schedule.cronHelp":
    "Pět polí: minuta, hodina, den v měsíci, měsíc, den v týdnu.",
  "script.schedule.cronUnreadable": "Tento výraz nelze popsat.",
  "script.schedule.invalid": "Neplatný plán",
  "script.schedule.nextRuns": "Příští spuštění",
  "settings.scriptInterpreters.title": "Interprety skriptů",
  "settings.scriptInterpreters.description":
    "Programy, kterými Scryer spouští skripty napsané v jednotlivých jazycích. Ponechte pole prázdné pro použití programu nalezeného v PATH.",
  "settings.scriptInterpreters.python": "Python",
  "settings.scriptInterpreters.powershell": "PowerShell",
  "settings.scriptInterpreters.batch": "Batch",
  "settings.scriptInterpreters.go": "Go",
  "settings.scriptInterpreters.containerHelp":
    "Obraz kontejneru Scryer neobsahuje žádné interprety. Nainstalujte ty, které potřebujete, a nasměrujte na ně tyto cesty.",
  "settings.scriptInterpreters.powershellHelp":
    "Skripty PowerShellu se spouští přes pwsh, což je PowerShell 7.",
  "settings.scriptInterpreters.saved": "Cesty k interpretům uloženy",
  "episodeAssign.action": "Přiřadit znovu",
  "episodeAssign.dialogTitle": "Přiřadit k epizodě",
  "episodeAssign.dialogDescription":
    "Zvolte epizodu, ke které {{name}} patří. Scryer soubor pro tuto epizodu přejmenuje a přesune na místo.",
  "episodeAssign.pendingDescription":
    "Scryer nedokázal poznat, ke které epizodě {{name}} patří. Zvolte epizodu a Scryer soubor tam naimportuje.",
  "episodeAssign.unnamed": "tento soubor",
  "episodeAssign.targetLabel": "Epizoda",
  "episodeAssign.loadingEpisodes": "Načítání epizod…",
  "episodeAssign.loadFailed": "Načtení epizod se nezdařilo.",
  "episodeAssign.unchanged": "Soubor je už přiřazen k této epizodě.",
  "episodeAssign.apply": "Přiřadit a přejmenovat",
  "episodeAssign.originalReleaseName": "Původní název vydání",
  "episodeAssign.applying": "Přiřazování…",
  "episodeAssign.replaceTitle": "Tato epizoda už má soubor",
  "episodeAssign.replaceDescription":
    "Přiřazením tohoto souboru se z něj stane hlavní soubor epizody. Současný soubor zůstane na disku jako doplňkový.",
  "episodeAssign.replaceApply": "Nahradit a přiřadit",
  "episodeAssign.success": "Soubor přiřazen a přejmenován.",
  "episodeAssign.failed": "Přiřazení souboru se nezdařilo.",
  "episodeAssign.seasonGroup": "Řada {{season}}",
  "episodeAssign.filterPlaceholder": "Filtrovat epizody…",
  "episodeAssign.filterLabel": "Filtrovat epizody",
  "episodeAssign.listLabel": "Epizody",
  "episodeAssign.noMatches": "Žádné odpovídající epizody.",
  "episodeAssign.selectPlaceholder": "Vyberte epizodu…",
  "episodeAssign.refusal.noEpisodes": "Zvolte alespoň jednu epizodu.",
  "episodeAssign.refusal.outsideTitle": "Tato epizoda patří k jinému titulu.",
  "episodeAssign.refusal.primaryConflict": "Tato epizoda už má hlavní soubor.",
  "episodeAssign.refusal.notEpisodic":
    "K epizodám lze přiřadit jen soubory seriálů a anime.",
  "episodeAssign.refusal.unsupportedFile":
    "Tento soubor nelze mezi epizodami přesouvat. Filmové soubory seriálů a obrazy disků zůstávají na svém místě.",
  "episodeAssign.refusal.destinationOccupied":
    "Jiný soubor už má název, který by tato epizoda vykreslila. Přesuňte ho nebo odeberte a zkuste to znovu.",
  "episodeAssign.refusal.renameRefused":
    "Scryer nemohl soubor pro tuto epizodu pojmenovat. Zkontrolujte složku knihovny titulu a šablonu pojmenování.",
  "episodeAssign.refusal.renameFailed":
    "Přesun souboru pod novým názvem se nezdařil. Soubor i jeho epizody zůstaly beze změny.",
  "pendingImports.reason.anime_numbering_ambiguous":
    "Číslování epizod je nejednoznačné",
};

export default cs;
