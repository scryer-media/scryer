import type { LocaleDictionary } from "../types";

const ru: LocaleDictionary = {
  "error.reason.METADATA_GATEWAY_CLOCK_SKEW":
    "Scryer не может подключиться к шлюзу метаданных, поскольку системные часы расходятся более чем на 5 минут. Синхронизируйте системные часы и перезапустите Scryer.",
  "httpRelease.refusal.detailsUnavailable":
    "Сведения об этом релизе больше недоступны. Повторите поиск, чтобы скачать его.",
  "httpRelease.refusal.live":
    "Этот релиз — прямая трансляция, её нельзя скачать.",
  "httpRelease.refusal.upcoming": "Этот релиз ещё не опубликован.",
  "httpRelease.refusal.removed": "Этот релиз удалён поставщиком.",
  "httpRelease.refusal.unsupportedTransport":
    "Этот релиз не предлагается в виде, который может получить HTTP-загрузчик.",
  "httpRelease.refusal.separateTracks":
    "Этот релиз предлагается только отдельными аудио- и видеодорожками, которые нельзя объединить.",
  "httpRelease.refusal.noMediaResource":
    "В этом релизе нет медиафайла для скачивания.",
  "httpRelease.refusal.noSubtitleResource":
    "Для этого релиза нет субтитров на нужном языке.",
  "titleDelete.refusal.selection": "Выберите названия без повторов.",
  "titleDelete.refusal.busy": "Удаление уже выполняется.",
  "titleDelete.refusal.preview":
    "Обновите и подтвердите предварительный просмотр удаления.",
  "titleDelete.refusal.confirmation":
    "Введите DELETE для подтверждения удаления.",
  "titleDelete.refusal.boundary":
    "Папка названия должна находиться внутри корня библиотеки, а не быть самим корнем.",
  "titleDelete.refusal.ownership":
    "Не удалось проверить принадлежность папки. Файлы сохранены.",
  "titleDelete.refusal.layout":
    "Папку или её содержимое нельзя безопасно удалить как одно название.",
  "titleDelete.refusal.unavailable":
    "Корень библиотеки недоступен. Восстановите доступ и обновите предварительный просмотр.",
  "titleDelete.refusal.outsideRoots":
    "Путь к файлу находится вне всех настроенных корней библиотеки. Исправьте корни библиотеки или папку тайтла и обновите предварительный просмотр.",
  "queue.attention.downloadPasswordRetry":
    "Повторная попытка ввода пароля для загрузки ожидает сверки. Удалите эту загрузку, если в клиенте больше нет задания.",
  "titleDelete.refusal.recycle":
    "Название удалено, но некоторые записи корзины не удалось очистить.",
  "titleDelete.refusal.retry":
    "Очистка каталога не удалась. Получите новый предварительный просмотр перед повтором.",
  "settings.downloadClientHttpDownloadDir": "Папка загрузок",
  "settings.downloadClientHttpDownloadDirHelp":
    "Куда записываются готовые файлы и их части (путь, как его видит Scryer). Очередь этого загрузчика хранится во вложенной папке .scryer-http.",
  "settings.downloadClientHttpMaxConcurrentDownloads": "Одновременные загрузки",
  "settings.downloadClientHttpMaxConcurrentDownloadsHelp":
    "Сколько релизов загружается одновременно (от 1 до 8).",
  "settings.downloadClientHttpMaxConnectionsPerHost": "Соединений на хост",
  "settings.downloadClientHttpMaxConnectionsPerHostHelp":
    "Сколько загрузок могут одновременно получать данные с одного сервера (от 1 до 8).",
  "settings.downloadClientHttpBandwidthLimit": "Ограничение скорости (КБ/с)",
  "settings.downloadClientHttpBandwidthLimitHelp":
    "Общее для всех загрузок этого клиента. Пусто или 0 — без ограничения.",
  "settings.publicUrlTitle": "Публичный URL",
  "settings.publicUrlHelp":
    "Адрес, по которому люди и сервисы обращаются к Scryer, например https://scryer.example.com. Привязка аккаунтов и OAuth применяют изменение сразу, а ключи доступа — после перезапуска Scryer.",
  "settings.publicUrlLabel": "Публичный URL",
  "settings.publicUrlEnvironmentLocked":
    "Задан через SCRYER_PUBLIC_URL. Чтобы изменить его, измените переменную окружения.",
  "settings.publicUrlInvalid":
    "Настроенный публичный URL недействителен и не используется: {{error}}",
  "settings.publicUrlError.invalid_url":
    "Введите абсолютный URL с http или https, например https://scryer.example.com.",
  "settings.publicUrlError.wildcard_host":
    "Публичный URL должен указывать один хост, а не шаблон.",
  "settings.publicUrlError.credentials":
    "Публичный URL не должен содержать имя пользователя или пароль.",
  "settings.publicUrlError.query_or_fragment":
    "Публичный URL не должен содержать строку запроса или фрагмент.",
  "settings.publicUrlError.path_not_allowed":
    "Scryer обслуживается из корня, поэтому публичный URL не должен содержать путь.",
  "settings.publicUrlError.path_mismatch":
    "Путь публичного URL должен быть пустым или совпадать с базовым путём Scryer.",
  "settings.publicUrlError.environment_locked":
    "Публичный URL задан через SCRYER_PUBLIC_URL и не может быть изменён здесь.",
  "settings.publicUrlError.save_and_reset":
    "Сохраните публичный URL или очистите его, но не одновременно.",
  "settings.publicUrlError.passkey_acknowledgement_required":
    "После этого изменения зарегистрированные ключи доступа перестанут работать. Подтвердите, чтобы сохранить.",
  "settings.publicUrlPasskeyRestart":
    "Ключи доступа учитывают изменение публичного URL после перезапуска Scryer.",
  "settings.publicUrlClear": "Очистить публичный URL",
  "settings.publicUrlSaved": "Публичный URL сохранён.",
  "settings.publicUrlLoadError": "Не удалось загрузить публичный URL.",
  "settings.publicUrlSaveError": "Не удалось сохранить публичный URL.",
  "settings.publicUrlEffectiveValues": "Действующие значения",
  "settings.publicUrlColumnSetting": "Параметр",
  "settings.publicUrlColumnValue": "Значение",
  "settings.publicUrlColumnSource": "Источник",
  "settings.publicUrlEffective": "Публичный URL",
  "settings.publicUrlBasePath": "Базовый путь",
  "settings.publicUrlBindAddress": "Адрес привязки",
  "settings.publicUrlTrustedProxies": "Доверенные прокси",
  "settings.publicUrlPasskeyRpId": "ID проверяющей стороны ключей доступа",
  "settings.publicUrlPasskeyRpOrigin": "Источник ключей доступа",
  "settings.publicUrlNotSet": "Не задано",
  "settings.publicUrlSourceEnvironment": "Окружение",
  "settings.publicUrlSourceSettings": "Сохранённые настройки",
  "settings.publicUrlSourceDefault": "По умолчанию",
  "settings.publicUrlSourcePublicUrl": "Публичный URL",
  "settings.publicUrlPasskeyWarningTitle":
    "Существующие ключи доступа перестанут работать",
  "settings.publicUrlPasskeyWarningChanged":
    "Ключи доступа зарегистрированы для {{rpId}}. После следующего перезапуска ключи доступа перейдут на {{nextRpId}}, и ни один существующий ключ больше не позволит войти.",
  "settings.publicUrlPasskeyWarningDisabled":
    "Ключи доступа зарегистрированы для {{rpId}}. Этот публичный URL не поддерживает ключи доступа (нужно доменное имя с https или localhost), поэтому после следующего перезапуска ключи доступа отключатся, и ни один существующий ключ больше не позволит войти.",
  "settings.publicUrlPasskeyWarningCounts":
    "Ключи доступа есть у {{users}} пользователей. У {{passkeyOnly}} из них нет приложения-аутентификатора в качестве второго фактора, и они потеряют доступ: войти не получится, пока их факторы не будут сброшены.",
  "settings.publicUrlPasskeyWarningCountsUnknown":
    "Scryer не удалось подсчитать зарегистрированные ключи доступа; считайте, что часть пользователей потеряет доступ.",
  "settings.publicUrlPasskeyWarningRecovery":
    "Чтобы восстановить доступ, администратор, который ещё может войти, может открыть «{{users}}» и выполнить «{{reset}}» для каждого заблокированного пользователя; либо верните прежний публичный URL и перезапустите Scryer; либо запустите Scryer с SCRYER_RECOVERY_ADMIN_PASSWORD, чтобы войти как администратор восстановления.",
  "settings.publicUrlPasskeyWarningConfirm": "Всё равно сохранить",
  "scoring.mandatoryRejection": "Обязательное требование не выполнено",
  "scoring.finalScoreRejection": "Итоговая оценка отклонена",
  "settings.arrImportBlockScoreHelp":
    "Итоговая оценка должна быть выше -9000 и не ниже минимума профиля. Обязательные требования сохраняются.",
  brand: "scryer",
  "label.ready": "Готово",
  "label.search": "Поиск",
  "label.autoSearch": "Автопоиск",
  "label.interactiveSearch": "Интерактивный поиск",
  "label.interactive": "Интерактивный",
  "label.searching": "Поиск...",
  "help.autoSearchTooltip":
    "Автоматически выполнить поиск и добавить в очередь последний подходящий релиз для этого элемента.",
  "help.interactiveSearchTooltip":
    "Выполнить поиск для этого элемента и открыть окно выбора релиза, чтобы добавить его в очередь.",
  "series.searchSeason": "Искать сезонный пакет",
  "help.seasonSearchTooltip":
    "Искать сезонный пакет для этого сезона в индексаторах.",
  "label.searchingWithQuery": "Поиск {{query}}",
  "label.refresh": "Обновить",
  "label.refreshing": "Обновление…",
  "label.save": "Сохранить",
  "label.saving": "Сохранение…",
  "label.saveOrder": "Сохранить порядок",
  "label.loading": "Загрузка…",
  "label.moveUp": "Переместить вверх",
  "label.moveDown": "Переместить вниз",
  "label.update": "Обновить",
  "label.name": "Имя",
  "label.load": "Загрузить",
  "label.edit": "Редактировать",
  "label.delete": "Удалить",
  "label.dismissActivityToast": "Закрыть уведомление об активности",
  "label.actions": "Действия",
  "label.create": "Создать",
  "label.cancel": "Отмена",
  "folderBrowser.selectPath": "Выбрать путь",
  "folderBrowser.go": "Перейти",
  "folderBrowser.descriptionFileOrFolder":
    "Просмотрите пути на хосте Scryer и выберите файл или папку.",
  "folderBrowser.descriptionFile":
    "Просмотрите пути на хосте Scryer и выберите файл.",
  "folderBrowser.descriptionFolder":
    "Просмотрите пути на хосте Scryer и выберите папку.",
  "folderBrowser.loading": "Загрузка папок",
  "folderBrowser.emptyFilesAndFolders": "Нет файлов или вложенных каталогов",
  "folderBrowser.emptyFolders": "Нет вложенных каталогов",
  "folderBrowser.error": "Не удалось открыть путь.",
  "folderBrowser.browseRoot": "Просмотреть /",
  "folderBrowser.selectFolder": "Выбрать папку",
  "folderBrowser.newFolder": "Новая папка",
  "folderBrowser.newFolderName": "Имя папки",
  "folderBrowser.create": "Создать",
  "folderBrowser.newFolderFailed": "Не удалось создать папку.",
  "error.reason.FOLDER_CREATE_NAME_REQUIRED": "Введите имя папки.",
  "error.reason.FOLDER_CREATE_NAME_RESERVED":
    "Это имя зарезервировано и не может использоваться для папки.",
  "error.reason.FOLDER_CREATE_NAME_INVALID_CHARACTERS":
    'Имя папки не может содержать / \\ : * ? " < > | или управляющие символы.',
  "error.reason.FOLDER_CREATE_NAME_TRAILING_DOT":
    "Имя папки не может заканчиваться точкой.",
  "error.reason.FOLDER_CREATE_NAME_HIDDEN":
    "Имя папки не может начинаться с точки, иначе папка будет скрытой.",
  "error.reason.FOLDER_CREATE_NAME_TOO_LONG":
    "Это имя слишком длинное для этого диска.",
  "error.reason.FOLDER_CREATE_NAME_NOT_ALLOWED":
    "Этот диск не принимает такое имя. Попробуйте более простое.",
  "error.reason.FOLDER_CREATE_PARENT_NOT_ABSOLUTE":
    "Выберите абсолютный путь к папке.",
  "error.reason.FOLDER_CREATE_PARENT_MISSING":
    "Текущая папка больше не существует или недоступна.",
  "error.reason.FOLDER_CREATE_PARENT_NOT_DIRECTORY":
    "Текущий путь указывает на файл, а не на папку.",
  "error.reason.FOLDER_CREATE_ALREADY_EXISTS":
    "Здесь уже есть файл или папка с таким именем. Некоторые диски не различают заглавные и строчные буквы.",
  "error.reason.FOLDER_CREATE_PERMISSION_DENIED":
    "У Scryer нет прав на создание папок здесь. Проверьте права на папку для пользователя, от имени которого работает Scryer (PUID и PGID в Docker).",
  "error.reason.FOLDER_CREATE_READ_ONLY":
    "Этот диск доступен только для чтения. Если Scryer работает в Docker, проверьте, что том не смонтирован только для чтения (:ro).",
  "error.reason.FOLDER_CREATE_DISK_FULL":
    "Диск заполнен или исчерпана квота хранилища.",
  "error.reason.FOLDER_CREATE_FOLDER_FULL":
    "В этой папке уже столько подпапок, сколько допускает её диск.",
  "error.reason.FOLDER_CREATE_NOT_SUPPORTED":
    "В этом месте нельзя создавать папки.",
  "error.reason.FOLDER_CREATE_DRIVE_UNAVAILABLE":
    "Диск или сетевая папка не отвечает. Проверьте, что они подключены и смонтированы.",
  "error.reason.FOLDER_CREATE_FAILED":
    "Scryer не удалось создать папку. Подробности в журнале сервера.",
  "label.discard": "Отменить изменения",
  "label.clear": "Очистить",
  "label.add": "Добавить",
  "label.remove": "Удалить",
  "label.or": "или",
  "label.yes": "Да",
  "label.no": "Нет",
  "label.category": "Категория",
  "label.filter": "Фильтр",
  "label.filters": "Фильтры",
  "label.for": "для",
  "label.enabled": "Включено",
  "label.disabled": "Отключено",
  "unit.seconds": "секунды",
  "label.enable": "Включить",
  "label.disable": "Отключить",
  "label.version": "Версия",
  "label.type": "Тип",
  "label.title": "Название",
  "label.status": "Статус",
  "label.testConnection": "Проверить соединение",
  "label.activityStream": "Лента активности",
  "external.openOn": "Открыть на {{site}}",
  "label.unknown": "Неизвестно",
  "label.unchanged": "Без изменений",
  "label.deleting": "Удаление…",
  "label.configured": "Настроено",
  "label.missing": "Отсутствует",
  "label.default": "по умолчанию",
  "label.language": "Язык",
  "label.movies": "Фильмы",
  "label.series": "Сериалы",
  "label.anime": "Аниме",
  "label.unknownType": "неизвестный тип",
  "label.yearUnknown": "год неизвестен",
  "label.noArt": "нет изображения",
  "label.emptyPermissions": "Нет разрешений",
  "label.none": "Нет",
  "nav.movies": "Фильмы",
  "nav.series": "Сериалы",
  "nav.anime": "Аниме",
  "nav.discovery": "Обзор",
  "nav.activity": "Активность",
  "nav.calendar": "Календарь",
  "nav.history": "История",
  "nav.settings": "Настройки",
  "nav.library": "Библиотека",
  "nav.catalog": "Каталог",
  "nav.import": "Импорт",
  "nav.system": "Система",
  "nav.logs": "Журналы",
  "nav.serviceLogs": "Журналы служб",
  "nav.auditLogs": "Журналы аудита",
  "nav.usersAccess": "Пользователи и доступ",
  "nav.group": "Навигация",
  "nav.mobileTitle": "Навигация Scryer",
  "nav.mobileDescription": "Основная навигация Scryer",
  "nav.mobileTrigger": "Открыть навигацию",
  "nav.group.catalogs": "Каталоги",
  "nav.group.discover": "Обзор",
  "nav.group.requests": "Запросы",
  "nav.group.automation": "Автоматизация",
  "nav.group.configuration": "Конфигурация",
  "nav.group.integrations": "Интеграции",
  "nav.group.system": "Система",
  "nav.group.more": "Дополнительно",
  "discovery.failedToLoad": "Не удалось загрузить раздел рекомендаций",
  "discovery.featured": "Рекомендуемое",
  "discovery.inLibrary": "В библиотеке",
  "discovery.add": "Добавить",
  "discovery.request": "Запросить",
  "discovery.requested": "Запрошено",
  "discovery.trailer": "Трейлер",
  "discovery.trailerUnavailable":
    "Трейлер недоступен в данных раздела рекомендаций",
  "discovery.openFilters": "Открыть фильтры",
  "discovery.closeFilters": "Закрыть фильтры",
  "discovery.tab.forYou": "Для вас",
  "discovery.tab.trending": "В тренде",
  "discovery.tab.popular": "Популярное",
  "discovery.tab.upcoming": "Скоро",
  "discovery.tab.topRated": "С высоким рейтингом",
  "discovery.filters": "Фильтры",
  "discovery.contentType": "Тип контента",
  "discovery.type.movies": "Фильмы",
  "discovery.type.series": "Сериалы",
  "discovery.type.anime": "Аниме",
  "discovery.genres": "Жанры",
  "discovery.selectGenres": "Выберите жанры...",
  "discovery.tags": "Теги",
  "discovery.selectTags": "Выберите теги...",
  "discovery.browseByGenre": "Просмотр по жанрам",
  "discovery.genreCount": "{{count}} произведений",
  "discovery.emptyTitle": "Пока нет рекомендаций",
  "discovery.emptyDescription":
    "После синхронизации здесь появятся рекомендации.",
  "discovery.trendingThisWeek": "В тренде на этой неделе",
  "discovery.viewAll": "Показать всё",
  "discovery.clearAll": "Очистить всё",
  "discovery.releaseYear": "Год выпуска",
  "discovery.minimumRating": "Минимальный рейтинг",
  "discovery.relationship": "Связь",
  "discovery.studio": "Студия",
  "discovery.section.newOnStreaming": "Новинки стриминговых сервисов",
  "discovery.section.newOnPhysical": "Новинки на Blu-ray",
  "discovery.section.allTimeFavorites": "Любимые за всё время",
  "discovery.section.rightNow": "Прямо сейчас",
  "discovery.section.fromYourTaste": "На основе ваших предпочтений",
  "discovery.section.almostComplete": "Почти завершено",
  "discovery.section.nextAnimeSeason": "Следующий сезон аниме",
  "discovery.section.popularInAnime": "Популярное в аниме",
  "discovery.section.thisSeasonInAnime": "Аниме текущего сезона",
  "discovery.section.comingSoonMovies": "Скоро выйдут: фильмы",
  "discovery.section.comingSoonSeries": "Скоро выйдут: сериалы",
  "discovery.section.topRatedForYou": "Лучшее для вас",
  "discovery.section.topRated": "С высоким рейтингом",
  "discovery.section.recommended": "Рекомендуемое",
  "discovery.recency.newThisWeek": "Новинки этой недели",
  "discovery.recency.newThisMonth": "Новинки этого месяца",
  "discovery.recency.new": "Новинка",
  "discovery.relation.sequel": "Продолжение",
  "discovery.relation.prequel": "Предыстория",
  "discovery.relation.sideStory": "Побочная история",
  "discovery.relation.spinOff": "Спин-офф",
  "discovery.relation.adaptation": "Адаптация",
  "discovery.relation.sharedUniverse": "Общая вселенная",
  "discovery.relation.alternative": "Альтернативная версия",
  "discovery.notInterested": "Не интересует",
  "discovery.hiddenTitles": "Скрытые произведения",
  "discovery.showHidden": "Показать скрытые",
  "discovery.updatedRelative": "Обновлено {{relative}}",
  "discovery.updatePending": "Ожидается обновление",
  "discovery.updatePendingHint":
    "Изменения в библиотеке появятся после следующей синхронизации.",
  "command.paletteTitle": "Перейти",
  "command.paletteDescription":
    "Поиск по разделам, настройкам и названиям каталога",
  "command.palettePlaceholder": "Введите название страницы или элемента",
  "command.paletteNoResults": "Подходящие разделы или элементы не найдены",
  "command.paletteGroup": "Навигация",
  settings: "Настройки",
  "settings.navGroup.account": "Учётная запись",
  "settings.navGroup.catalogs": "Каталоги",
  "settings.navGroup.integrations": "Интеграции",
  "settings.navGroup.system": "Система",
  "settings.profile": "Профиль",
  "settings.general": "Общие",
  "settings.backups": "Резервные копии",
  "settings.security": "Безопасность",
  "settings.securityLoginSettings": "Настройки входа",
  "settings.securityEnableFormLoginHelp":
    "Если параметр включён, Scryer требует аутентифицированный сеанс пользователя. Если параметр отключён, запросы выполняются от имени администратора по умолчанию.",
  "settings.securityPasswordMinLength": "Минимальная длина пароля",
  "settings.securitySessionDuration": "Вход действителен",
  "settings.securitySessionDurationHelp":
    "Применяется к новым входам всех пользователей этого экземпляра. Если включено сохранение входа, сеансы сохраняются после перезапуска браузера до истечения срока. Срок существующих сеансов не меняется.",
  "settings.securitySessionDurationInvalid":
    "Длительность сеанса должна быть целым числом от 1 до 365 дней.",
  "settings.securityPasswordMinLengthHelp":
    "Применяется только к новым или изменённым паролям Scryer. Существующие более короткие пароли продолжат работать. Минимум: {{min}}.",
  "settings.securityPasswordMinLengthInvalid":
    "Минимальная длина пароля должна быть не менее {{min}}.",
  "settings.manageMediaServerLogins": "Управление входом на медиасервер",
  "settings.manageMediaServerLoginsDescription":
    "Настройка входа в Jellyfin, привязки учётных записей и автоматического создания пользователей при подключении к медиасерверам.",
  "settings.openMediaServers": "Открыть медиасерверы",
  "settings.securitySkipLocalIps": "Пропускать локальные IP-адреса",
  "settings.securitySkipLocalIpsHelp":
    "Если параметр включён, пользователи из диапазонов 10.0.0.0/8, 172.16.0.0/12 и 192.168.0.0/16 смогут получать доступ к Scryer без входа в систему.",
  "settings.securityMfaConfigStepUp":
    "Требовать MFA для изменения конфигурации",
  "settings.securityMfaConfigStepUpHelp":
    "Если параметр включён, изменение системных настроек и конфигурации компонентов потребует недавнего подтверждения MFA.",
  "settings.securityMfaPasswordLogin": "Требовать MFA при входе по паролю",
  "settings.securityMfaPasswordLoginHelp":
    "Если параметр включён, для входа в Scryer по имени пользователя и паролю потребуется одноразовый код. Использование ключей доступа не изменится.",
  "settings.securityTotpJellyfinLogin":
    "Требовать MFA для входа через Jellyfin",
  "settings.securityTotpJellyfinLoginHelp":
    "Если параметр включён, вход через передачу пароля Jellyfin потребует MFA-код пользователя Scryer.",
  "settings.securitySavedPreference": "Сохранённый параметр",
  "settings.securityEffectiveMode": "Текущий режим работы",
  "settings.securityOverrideTitle": "Переопределение конфигурации активно",
  "settings.securityOverrideDescription":
    "Конфигурация окружения в настоящее время переопределяет сохранённый параметр. Измените конфигурацию развёртывания и перезапустите Scryer, чтобы применить другой режим работы.",
  "settings.securityOverrideReason": "Источник переопределения: {{override}}",
  "settings.securityModeEnabled": "Вход через форму включён",
  "settings.securityModeDisabled": "Открытый режим администратора",
  "settings.securityConfirmTitle": "Включить вход через форму?",
  "settings.securityConfirmDescription":
    "Введите пароль, прежде чем Scryer начнёт требовать вход в систему.",
  "settings.securityConfirmPassword": "Пароль",
  "settings.securityConfirmAction": "Включить",
  "settings.securityAdminPasswordRequiredTitle":
    "Сначала задайте пароль администратора",
  "settings.securityAdminPasswordRequiredDescription":
    "Для входа через форму у учётной записи администратора должен быть установлен пароль. Откройте свой профиль, чтобы задать первоначальный пароль, а затем вернитесь сюда и включите вход через форму.",
  "settings.securityAdminPasswordRequiredAction": "Открыть профиль",
  "settings.securityDisableConfirmTitle": "Отключить вход через форму?",
  "settings.securityDisableConfirmDescription":
    "После перезагрузки Scryer вернётся в открытый режим администратора.",
  "settings.securityDisableConfirmAction": "Отключить",
  "settings.securityCredentialsInvalid": "Неверный пароль.",
  "settings.securityCredentialsInsufficient":
    "У этой учётной записи нет прав на управление настройками.",
  "settings.mfaStepUpTitle": "Подтвердите доступ к настройкам",
  "settings.mfaStepUpDescription":
    "Введите код из приложения-аутентификатора перед просмотром или изменением настроек конфигурации.",
  "settings.mfaStepUpSubmit": "Продолжить",
  "settings.mfaStepUpVerifying": "Проверка...",
  "settings.mfaStepUpVerified": "Доступ к настройкам подтверждён.",
  "settings.mfaStepUpFailed": "Не удалось проверить код. Попробуйте ещё раз.",
  "settings.mfaStepUpRequiredAgain":
    "Срок действия подтверждения доступа к настройкам истёк. Введите новый код из приложения-аутентификатора, чтобы продолжить.",
  "settings.mfaStepUpPolicyLoadFailed":
    "Не удалось загрузить статус проверки доступа к настройкам.",
  "settings.mfaStepUpPolicyLoadFailedDescription":
    "Повторите попытку перед просмотром или изменением настроек конфигурации.",
  "settings.mfaStepUpPolicyRetry": "Повторить",
  "settings.securityEnabledSuccess": "Вход через форму включён.",
  "settings.securityDisabledSuccess":
    "Вход через форму отключён. Выполняется переход в открытый режим администратора…",
  "settings.securityPreferenceSaved": "Параметры безопасности сохранены.",
  "settings.securityLoadFailed": "Не удалось загрузить настройки безопасности.",
  "settings.securitySaveFailed": "Не удалось сохранить настройки безопасности.",
  "settings.externalAuthLoginEnabled": "Вход включён",
  "settings.mediaServerLoginRequiresFormLogin":
    "Включите вход через форму перед активацией входа через медиасервер.",
  "settings.openSecuritySettings": "Открыть настройки безопасности",
  "settings.externalAuthLinkingEnabled": "Связывание включено",
  "settings.addConnection": "Добавить",
  "settings.removeConnection": "Удалить подключение",
  "settings.noConnectionsConfigured": "Подключения не настроены.",
  "settings.connectionId": "Идентификатор подключения",
  "settings.connectionName": "Название",
  "settings.connectionBaseUrl": "Базовый URL",
  "settings.plexMachineId": "Идентификатор устройства",
  "settings.externalAccountInvites": "Приглашения для внешних учётных записей",
  "settings.externalAccountInviteRequired":
    "Выберите пользователя и учётную запись медиасервера.",
  "settings.externalAccountInviteCreated":
    "Приглашение для внешней учётной записи создано.",
  "settings.externalAccountInviteFailed":
    "Не удалось создать приглашение для внешней учётной записи.",
  "settings.mediaServerUser": "Пользователь медиасервера",
  "settings.mediaServerUserSearchPlaceholder":
    "Поиск пользователей медиасервера",
  "settings.mediaServerUserSearchFailed":
    "Ошибка поиска пользователей медиасервера.",
  "settings.mediaServerUserPickerEmpty":
    "Пользователи медиасервера не найдены.",
  "settings.mediaServerUserPickerServerEmpty":
    "На этом сервере пользователи не найдены.",
  "settings.jellyfinUserSearchFailed": "Ошибка поиска пользователей Jellyfin.",
  "settings.jellyfinUserPickerEmpty": "Пользователи Jellyfin не найдены.",
  "settings.jellyfinUserPickerManualEntry": 'Использовать "{{value}}"',
  "settings.previousExternalAccountInvites": "Предыдущие приглашения",
  "settings.noExternalAccountInvites":
    "Приглашений для внешних учётных записей пока нет.",
  "settings.externalAccountInviteStatusDisabled": "Отключено",
  "settings.externalAccountInviteStatusLoggedIn": "Выполнен вход",
  "settings.externalAccountInviteStatusPending": "Ожидание",
  "settings.externalAccountInviteStatusActive": "Активно",
  "settings.externalAccountInviteProviderUser": "Пользователь поставщика",
  "settings.externalAccountInviteCreatedAt": "Создано",
  "settings.externalAccountInviteLastLogin": "Последний вход",
  "settings.provider": "Поставщик",
  "settings.jellyfinUsername": "Имя пользователя Jellyfin",
  "settings.plexUserId": "Идентификатор пользователя Plex",
  "settings.createInvite": "Создать приглашение",
  "settings.users": "Пользователи",
  "settings.indexers": "Индексаторы",
  "settings.indexerSearch": "Поиск по индексаторам",
  "settings.proxyUsername": "Имя пользователя",
  "settings.proxyPassword": "Пароль",
  "settings.proxyCredentialsStored": "Учётные данные заданы",
  "settings.proxyCredentialUnchanged": "Без изменений",
  "settings.proxyClearCredentials": "Удалить сохранённые учётные данные",
  "settings.proxyCredentialsHelp":
    "Учётные данные хранятся в зашифрованном виде и больше не показываются. Оставьте оба поля пустыми, если прокси их не требует.",
  "settings.proxyCredentialsStoredHelp":
    "Для этого прокси сохранены учётные данные. Оставьте поля пустыми, чтобы сохранить их, или введите новые значения для замены.",
  "settings.proxyRemoteDns": "Разрешать DNS на прокси",
  "settings.proxyRemoteDnsHelp":
    "Разрешение DNS на прокси — это поведение socks5h: имена узлов назначения разрешает прокси, а не эта машина.",
  "settings.proxies": "Прокси",
  "settings.proxiesHelp":
    "Прокси передают трафик тех индексаторов и клиентов загрузки, которым они назначены. Решатель отвечает на браузерные проверки, обычный прокси — это узел HTTP или SOCKS, а туннель поднимает собственный транспорт.",
  "settings.proxyFamilySolver": "Решатель",
  "settings.proxyFamilyStandard": "Обычные",
  "settings.proxyFamilyTunnel": "Туннель",
  "settings.proxyFamilyOther": "Прочее",
  "settings.proxyHealth": "Состояние",
  "settings.proxyLastError": "Последняя ошибка",
  "settings.proxyHealthHealthy": "Исправен",
  "settings.proxyHealthUnhealthy": "Неисправен",
  "settings.proxyEmpty": "Прокси не настроены.",
  "settings.proxyTest": "Проверить",
  "settings.proxyTimeout": "Тайм-аут",
  "settings.proxyCreateNew": "Подключить прокси",
  "settings.proxyCreate": "Создать прокси",
  "settings.proxyUpdate": "Обновить прокси",
  "settings.proxyDeleteConfirmDescription": "Удалить прокси {{name}}?",
  "settings.proxyValidation": "Требуются имя и базовый URL.",
  "settings.proxyEndpoint": "Конечная точка",
  "settings.proxyEndpointHelp":
    "ssh://хост:порт. Если порт не указан, используется 22.",
  "settings.proxyAssignment": "Прокси",
  "settings.proxyDirect": "Напрямую (без прокси)",
  "settings.proxyMissing": "Прокси не найден",
  "settings.proxyMissingHelp": "Назначенный прокси не найден.",
  "settings.proxyDisabledHelp": "Назначенный прокси отключён.",
  "settings.proxyDisabledSuffix": "(отключён)",
  "settings.proxyClearPassword": "Удалить сохранённый пароль",
  "settings.proxyTunnelAuthHelp":
    "Для SSH нужны имя пользователя и закрытый ключ Ed25519. Аутентификация по паролю не поддерживается.",
  "settings.proxyPrivateKey": "Закрытый ключ",
  "settings.proxyPrivateKeyHelp":
    "поддерживаются только закрытые ключи Ed25519; создайте ключ командой `ssh-keygen -t ed25519` и вставьте закрытый ключ в формате OpenSSH",
  "settings.proxyPrivateKeyStored": "Закрытый ключ задан",
  "settings.proxyPrivateKeyUnchanged":
    "Закрытый ключ сохранён. Вставьте новый, чтобы заменить его.",
  "settings.proxyClearPrivateKey": "Удалить сохранённый закрытый ключ",
  "settings.proxyPrivateKeyPassphrase": "Пароль ключа",
  "settings.proxyPrivateKeyPassphraseHelp":
    "Нужен, только если закрытый ключ защищён паролем.",
  "settings.proxyHostKey": "Ключ хоста",
  "settings.proxyHostKeyPinnedAt": "Закреплён {{time}}.",
  "settings.proxyHostKeyUnpinned":
    "Ещё не закреплён. Первое успешное подключение закрепит ключ хоста сервера.",
  "settings.proxyHostKeyReset": "Сбросить ключ хоста",
  "settings.proxyHostKeyResetDescription":
    "Забыть закреплённый ключ хоста. Следующее подключение доверится ключу, который предложит сервер, и закрепит его, поэтому делайте это, только если знаете, что ключ сервера действительно изменился.",
  "settings.proxyValidationTunnelUsername": "Туннелю нужно имя пользователя.",
  "settings.proxyValidationTunnelAuth":
    "Для SSH-туннеля нужен закрытый ключ Ed25519.",
  "settings.downloadClientProxyHelp":
    "Назначить можно прокси любого вида. Решатель проверок не влияет на встроенный клиент, запросы которого отправляет не плагин.",
  "status.proxyCreated": "Прокси создан.",
  "status.proxyUpdated": "Прокси обновлён.",
  "status.proxyDeleted": "Прокси {{name}} удалён.",
  "status.proxyTestPassed": "Проверка прокси пройдена.",
  "status.proxyTestFailed": "Проверка прокси не пройдена.",
  "status.editingProxy": "Редактирование прокси {{name}}.",
  "status.proxyHostKeyReset":
    "Ключ хоста для {{name}} сброшен. Следующее подключение закрепит его заново.",
  "settings.proxyEndpointHelpWireguard":
    "wireguard://хост:порт. Если порт не указан, используется 51820.",
  "settings.proxyPrivateKeyHelpWireguard":
    "Ключи WireGuard — это 32 байта в base64, ровно в том виде, в каком их печатает `wg genkey` (44 символа, оканчивающиеся на `=`)",
  "settings.proxyPeerPublicKey": "Открытый ключ узла",
  "settings.proxyPeerPublicKeyHelp":
    "Строка `PublicKey` из раздела `[Peer]` на сервере. Открытый ключ не является секретом, поэтому показывается целиком.",
  "settings.proxyPresharedKey": "Общий ключ",
  "settings.proxyPresharedKeyHelp":
    "Необязательно. Строка `PresharedKey`, если сервер её использует.",
  "settings.proxyPresharedKeyStored": "Общий ключ задан",
  "settings.proxyPresharedKeyUnchanged":
    "Общий ключ сохранён. Вставьте новый, чтобы заменить его.",
  "settings.proxyClearPresharedKey": "Удалить сохранённый общий ключ",
  "settings.proxyTunnelAddresses": "Адреса",
  "settings.proxyTunnelAddressesHelp":
    "Строка `Address` из раздела `[Interface]`, например 10.6.0.2/32. По одному в строке или через запятую.",
  "settings.proxyTunnelDnsServers": "DNS-серверы",
  "settings.proxyTunnelDnsServersHelp":
    "Необязательные резолверы, доступные через туннель: по одному в строке или через запятую. Без них к назначению придётся обращаться по IP.",
  "settings.proxyTunnelMtu": "MTU",
  "settings.proxyTunnelMtuHelp":
    "Необязательно, от {{min}} до {{max}}. Оставьте пустым, чтобы использовать значение по умолчанию {{default}}.",
  "settings.proxyTunnelKeepalive": "Поддержание соединения (Keepalive)",
  "settings.proxyTunnelKeepaliveHelp":
    "Необязательно, в секундах. Оставьте пустым, чтобы использовать значение по умолчанию {{default}}, или укажите 0, чтобы отключить keepalive.",
  "settings.proxyTunnelPublicKey": "Открытый ключ этого туннеля",
  "settings.proxyTunnelPublicKeyHelp":
    "Вставьте это в раздел `[Peer]` на сервере как строку `PublicKey`.",
  "settings.proxyTunnelPublicKeyPending":
    "Появится после сохранения закрытого ключа.",
  "settings.proxyTunnelPublicKeyCopy": "Копировать",
  "settings.proxyImportConfig": "Импорт конфигурации WireGuard",
  "settings.proxyImportConfigHelp":
    "Вставьте всю конфигурацию, которую дал вам VPN-провайдер, или выберите файл .conf — поля ниже будут заполнены из неё. Каждое поле также принимает одну вставленную строку.",
  "settings.proxyImportConfigApply": "Заполнить форму",
  "settings.proxyImportConfigFile": "Выбрать файл",
  "status.proxyImportConfigFilled":
    "Форма заполнена из конфигурации WireGuard.",
  "status.proxyImportConfigUnreadable":
    "Это не похоже на конфигурацию WireGuard.",
  "status.proxyImportConfigFirstPeer":
    "В ней {{count}} пиров; импортирован только первый.",
  "status.proxyImportConfigIgnored":
    "{{keys}} проигнорировано: туннельный прокси это не использует.",
  "settings.proxyValidationWireguardPrivateKey":
    "WireGuard требует закрытый ключ.",
  "settings.proxyValidationWireguardPeerPublicKey":
    "WireGuard требует открытый ключ узла.",
  "settings.proxyValidationWireguardAddresses":
    "WireGuard требует хотя бы один адрес интерфейса, например 10.6.0.2/32.",
  "settings.proxyValidationWireguardKeyShape":
    "{{field}} не похож на ключ WireGuard: это 32 байта в base64, ровно в том виде, в каком их печатает `wg genkey` (44 символа, оканчивающиеся на `=`).",
  "settings.proxyValidationWireguardMtu":
    "MTU туннеля должен быть от {{min}} до {{max}}.",
  "settings.proxyValidationWireguardKeepalive":
    "Keepalive — целое число секунд; 0 отключает его.",
  "status.proxyTunnelPublicKeyCopied": "Открытый ключ скопирован.",
  "status.proxyTunnelPublicKeyCopyFailed":
    "Не удалось скопировать открытый ключ.",
  "settings.qualityProfiles": "Профили качества",
  "settings.downloadClients": "Клиенты загрузки",
  "settings.mediaServers": "Медиасерверы",
  "settings.downloadClientRouting": "Маршрутизация клиентов загрузки",
  "settings.moviesSettings": "Настройки фильмов",
  "settings.seriesSettings": "Настройки сериалов",
  "settings.animeSettings": "Настройки аниме",
  "settings.mediaSettings": "Настройки медиаконтента",
  "facetSettings.general": "Общие",
  "facetSettings.quality": "Качество",
  "facetSettings.renaming": "Переименование",
  "facetSettings.routing": "Маршрутизация",
  "facetSettings.generalPolicies": "Политики",
  "facetSettings.sidecarFiles": "Сопутствующие файлы",
  "facetSettings.scoringPersona": "Профиль оценки",
  "facetSettings.scoringPersonaOverrideLabel": "Переопределение профиля",
  "facetSettings.scoringPersonaUseDefault": "Использовать профиль по умолчанию",
  "facetSettings.scoringBehavior": "Поведение оценки",
  "facetSettings.scoringBehaviorHint":
    "Определяет способ оценки релизов во время поиска. Зависит от выбранного профиля. Для более точной настройки измените профиль качества.",
  "facetSettings.effectiveScoringOverrides": "Переопределения оценки",
  "facetSettings.effectiveScoringOverridesHint":
    "Дополнительные параметры, применяемые поверх выбранного профиля. Измените профиль качества, чтобы настроить их.",
  "persona.trait.balanced.source":
    "Источник: стандартные предпочтения (BluRay > WEB-DL > WEBRip)",
  "persona.trait.balanced.audio": "Аудио: сбалансированная оценка кодеков",
  "persona.trait.balanced.x265":
    "x265: пониженный приоритет для разрешений ниже 4K",
  "persona.trait.balanced.size":
    "Размер файла: предпочтение отдаётся более крупным файлам",
  "persona.trait.balanced.remux": "Remux: по умолчанию не является приоритетом",
  "persona.trait.balanced.hdr":
    "HDR: Dolby Vision и HDR10 получают преимущество",
  "persona.trait.audiophile.source": "Источник: BluRay имеет высокий приоритет",
  "persona.trait.audiophile.audio":
    "Аудио: кодеки без потерь имеют высокий приоритет (TrueHD, DTS-X, FLAC)",
  "persona.trait.audiophile.x265":
    "x265: значительно снижает оценку для разрешений ниже 4K",
  "persona.trait.audiophile.size":
    "Размер файла: предпочтение отдаётся более крупным файлам",
  "persona.trait.audiophile.remux": "Remux: очень высокий приоритет",
  "persona.trait.audiophile.hdr":
    "HDR: Dolby Vision получает максимальный приоритет",
  "persona.trait.efficient.source":
    "Источник: предпочтение отдаётся WEB-DL и WEB-Rip",
  "persona.trait.efficient.audio":
    "Аудио: допускаются кодеки с потерями (предпочтение отдаётся DDP)",
  "persona.trait.efficient.x265":
    "x265: получает преимущество при любом разрешении",
  "persona.trait.efficient.size":
    "Размер файла: предпочтение отдаётся компактным версиям",
  "persona.trait.efficient.remux": "Remux: не является приоритетом",
  "persona.trait.efficient.hdr": "HDR: стандартные настройки",
  "persona.trait.compatible.source": "Источник: предпочтение отдаётся WEB-DL",
  "persona.trait.compatible.audio":
    "Аудио: DDP и AAC имеют приоритет для максимальной совместимости устройств",
  "persona.trait.compatible.x265": "x265: H.264 предпочтительнее H.265",
  "persona.trait.compatible.size": "Размер файла: стандартные предпочтения",
  "persona.trait.compatible.remux": "Remux: не является приоритетом",
  "persona.trait.compatible.hdr":
    "HDR: Dolby Vision получает штраф (риск несовместимости)",
  "settings.moviesLibrarySettings": "Настройки медиатеки фильмов",
  "settings.seriesLibrarySettings": "Настройки медиатеки сериалов",
  "libraryFilter.all": "Все библиотеки",
  "libraryFilter.selectedCount": "{{count}} библиотек",
  "settings.librariesLabel": "Библиотеки",
  "settings.libraryNameLabel": "Название библиотеки",
  "settings.libraryNamePlaceholder": "Детские",
  "settings.libraryNew": "Новая библиотека",
  "settings.libraryNewButton": "Новая библиотека",
  "settings.libraryCreateButton": "Создать библиотеку",
  "settings.librarySaveButton": "Сохранить библиотеку",
  "settings.librarySaveAndScanButton": "Сохранить и сканировать",
  "settings.librarySaveOnlyButton": "Только сохранить",
  "settings.libraryDeleteButton": "Удалить библиотеку",
  "settings.libraryDeleteConfirm":
    "Удалить библиотеку {{name}}? Медиафайлы на диске удалены не будут.",
  "settings.libraryEmpty": "Нет доступных библиотек.",
  "settings.libraryCreated": "Библиотека создана.",
  "settings.librarySaved": "Библиотека сохранена.",
  "settings.libraryDeleted": "Библиотека удалена.",
  "settings.librarySaveFailed": "Не удалось сохранить библиотеку.",
  "settings.libraryDeleteFailed": "Не удалось удалить библиотеку.",
  "settings.librarySettingsLoadFailed":
    "Не удалось загрузить настройки библиотеки.",
  "settings.libraryRequiredAudioLabel": "Требуемый аудиоформат",
  "settings.libraryEffectiveAudio": "Используется: {{value}}",
  "settings.libraryQualityProfileLabel": "Профиль качества",
  "settings.libraryRequestQualityProfilesLabel": "Профили запросов",
  "settings.libraryRequestQualityProfilesHelp":
    "Оставьте поле пустым, чтобы использовать профиль качества библиотеки для запросов.",
  "settings.libraryInheritFacet": "Наследовать настройки каталога по умолчанию",
  "settings.libraryCustomRouting": "Пользовательская маршрутизация библиотеки",
  "settings.libraryEffectiveProfile": "Используется: {{value}}",
  "settings.libraryScoringPersonaLabel": "Профиль оценки",
  "settings.libraryEffectivePersona": "Используется: {{value}}",
  "settings.rootFoldersLabel": "Корневые папки",
  "settings.rootFoldersEmpty":
    "Корневые папки не настроены. Добавьте хотя бы один путь.",
  "settings.rootFoldersHelp":
    "Корневые папки — это каталоги верхнего уровня, в которых хранятся медиафайлы. Папка по умолчанию используется для новых материалов.",
  "settings.rootFolderAdd": "Добавить корневую папку",
  "settings.rootFolderEdit": "Изменить корневую папку",
  "settings.rootFolderConflict":
    "Эта корневая папка уже используется в {{libraries}}.",
  "settings.rootFolderAddPlaceholder": "/data/movies",
  "settings.rootFolderSetDefault": "Использовать по умолчанию",
  "settings.rootFolderInvalid": "Недействительна",
  "settings.rootFolderInvalidTooltip":
    "Scryer не удалось получить доступ к этой корневой папке на хосте.",
  "settings.rootFolderValidationUnavailable":
    "Проверка корневых папок временно недоступна. Сохранённые пути не были помечены как недействительные.",
  "settings.unsavedLibraryChangesTitle": "Несохранённые изменения библиотеки",
  "settings.unsavedLibraryChangesConfirm":
    "Отменить несохранённые изменения библиотеки и покинуть эту страницу?",
  "settings.libraryScanTitle": "Сканирование библиотеки",
  "settings.libraryScanHelp":
    "Просканируйте папку с фильмами и импортируйте отсутствующие материалы.",
  "settings.libraryScanButton": "Сканировать библиотеку",
  "settings.libraryScanRunning": "Сканирование библиотеки…",
  "settings.libraryScanSummary":
    "{{imported}} импортировано, {{skipped}} пропущено, {{unmatched}} не сопоставлено.",
  "settings.libraryScanSuccess":
    "Сканирование библиотеки завершено. Импортировано: {{imported}}, пропущено: {{skipped}}, не сопоставлено: {{unmatched}}.",
  "settings.libraryScanFailed": "Ошибка сканирования библиотеки.",
  "settings.libraryScanCancel": "Отмена",
  "settings.libraryScanRunInBackground": "Выполнить в фоновом режиме",
  "settings.libraryScanCancelFailed":
    "Не удалось отменить сканирование библиотеки.",
  "settings.libraryScanCanceled": "Сканирование библиотеки отменено.",
  "settings.libraryScanCanceledSummary":
    "Сканирование библиотеки отменено. Импортировано: {{imported}}, пропущено: {{skipped}}, не сопоставлено: {{unmatched}}.",
  "settings.libraryScanToastTitle": "Сканирование библиотеки {{facet}}",
  "settings.libraryScanFoundTitles": "Найдено материалов: {{count}}",
  "settings.libraryScanDiscovering": "Поиск материалов…",
  "settings.libraryScanProgressPending": "Ожидание обнаружения…",
  "settings.libraryScanProgressCalculatingTotal": "Подсчёт общего количества…",
  "settings.libraryScanProgressScannedCount": "Просканировано: {{count}}",
  "settings.libraryScanTitleMatch": "Сопоставление названий",
  "settings.libraryScanFetchingMetadata": "Получение метаданных",
  "settings.libraryScanFilesScanned": "Анализ медиафайлов",
  "settings.libraryScanNoTitleMatchNeeded":
    "Сопоставление названий не требуется",
  "settings.libraryScanNoMetadataNeeded": "Получение метаданных не требуется",
  "settings.libraryScanNoFilesToScan": "Анализ медиафайлов не требуется",
  "settings.libraryScanAlreadyRunning":
    "Сканирование библиотеки {{facet}} уже выполняется.",
  "settings.libraryScanProgressCount": "{{current}} / {{total}}",
  "settings.libraryScanScanningSubtitle":
    "Найдено материалов: {{count}} · выполняется анализ…",
  "settings.libraryScanDoneSubtitle":
    "Сканирование завершено · {{count}} материалов",
  "settings.libraryScanIssuesSubtitle":
    "Завершено с ошибками · {{count}} материалов",
  "settings.libraryScanFailedSubtitle": "Сканирование завершилось ошибкой",
  "settings.libraryScanCanceledSubtitle": "Сканирование отменено",
  "settings.libraryScanBadgeDone": "Завершено",
  "settings.libraryScanBadgeReview": "Проверить",
  "settings.libraryScanBadgeFailed": "Ошибка",
  "settings.libraryScanBadgeCanceled": "Отменено",
  "settings.libraryScanViewTitles": "Показать материалы: {{count}}",
  "settings.libraryScanReviewUnmatched":
    "Проверить несопоставленные: {{count}}",
  "settings.libraryScanCompleted": "Сканирование библиотеки завершено.",
  "settings.libraryScanCompletedWithWarnings":
    "Сканирование библиотеки завершено с предупреждениями.",
  "jobs.title": "Задания",
  "jobs.activeRuns": "Активные задания",
  "jobs.primary": "Основные задания",
  "jobs.maintenance": "Обслуживание",
  "jobs.schedule": "Расписание",
  "jobs.never": "Никогда",
  "jobs.completed": "Завершено",
  "jobs.running": "Выполняется",
  "jobs.runSummaryCompleted": "Завершено",
  "jobs.runSummaryRunning": "Выполняется",
  "jobs.recentRuns": "Недавние запуски",
  "jobs.loadingRecentRuns": "Загрузка недавних запусков…",
  "jobs.noRunsYet": "Запуски пока не зарегистрированы.",
  "jobs.healthCheckIssues": "Проблемы проверки состояния",
  "jobs.fullHashBackfillFailures":
    "Файлы, для которых не удалось вычислить хеш",
  "jobs.fullHashBackfillFailuresMore": "…и ещё {{count}} не показано",
  "jobs.nextRunPrefix": "Следующий запуск: {{value}}",
  "jobs.startedAt": "Запущено {{value}}",
  "jobs.completedAt": "Завершено {{value}}",
  "jobs.failedToTrigger": "Не удалось запустить задание",
  "jobs.column.name": "Название",
  "jobs.column.category": "Категория",
  "jobs.column.schedule": "Расписание",
  "jobs.column.nextRun": "Следующий запуск",
  "jobs.column.lastRun": "Последний запуск",
  "jobs.column.status": "Статус",
  "jobs.column.trigger": "Запуск",
  "jobs.action.run": "Запустить",
  "jobs.action.runNow": "Запустить сейчас",
  "jobs.cancelSearchRequested":
    "Запрошена отмена. Текущий шаг поиска завершится, после чего остальные шаги остановятся.",
  "jobs.cancelSearchNotRunning": "Этот поиск уже не выполняется.",
  "jobs.cancelSearchFailed": "Не удалось отменить поиск.",
  "jobs.action.running": "Выполняется…",
  "jobs.status.idle": "Ожидание",
  "jobs.status.queued": "В очереди",
  "jobs.status.discovering": "Поиск",
  "jobs.status.running": "Выполняется",
  "jobs.status.completed": "Завершено",
  "jobs.status.warning": "Предупреждение",
  "jobs.status.failed": "Ошибка",
  "jobs.category.library": "Библиотека",
  "jobs.category.acquisition": "Получение",
  "jobs.category.maintenance": "Обслуживание",
  "jobs.category.subtitles": "Субтитры",
  "jobs.category.system": "Система",
  "jobs.triggerSource.manual": "Вручную",
  "jobs.triggerSource.scheduledStartup": "Запуск по расписанию при старте",
  "jobs.triggerSource.scheduledInterval": "Запуск по интервалу",
  "jobs.triggerSource.scheduledDaily": "Ежедневный запуск",
  "jobs.triggerSource.systemInternal": "Внутренний системный процесс",
  "deletePreview.counting": "Подсчёт файлов для удаления...",
  "deletePreview.error":
    "Не удалось подготовить предварительный просмотр удаления.",
  "deletePreview.folderUnknown":
    "Scryer не может определить, в какой папке лежат файлы этого тайтла, поэтому ничего не удалит с диска.",
  "deletePreview.removeFromCatalogOnly": "Убрать только из каталога",
  "deletePreview.files": "Файлы",
  "deletePreview.media": "Медиафайлы",
  "deletePreview.subtitles": "Субтитры",
  "deletePreview.images": "Изображения",
  "deletePreview.other": "Другое",
  "deletePreview.folders": "Папки",
  "deletePreview.samplePaths": "Примеры путей",
  "deletePreview.confirmPrompt": "Введите DELETE для подтверждения удаления.",
  "settings.saveButton": "Сохранить",
  "settings.comingSoon":
    "Этот раздел станет доступен в одном из следующих выпусков.",
  "settings.moviesPathLabel": "Путь к папке с фильмами",
  "settings.moviesPathHelp":
    "Используется как место назначения для выбранных релизов фильмов.",
  "settings.moviesPathPlaceholder": "/data/movies",
  "settings.moviesPathRequired": "Необходимо указать путь к папке с фильмами.",
  "settings.seriesPathLabel": "Путь к папке с сериалами",
  "settings.seriesPathHelp":
    "Используется как место назначения для выбранных релизов сериалов и аниме.",
  "settings.seriesPathPlaceholder": "/data/series",
  "settings.seriesPathRequired": "Необходимо указать путь к папке с сериалами.",
  "settings.movieSettingsSaved": "Настройки фильмов сохранены.",
  "settings.seriesSettingsSaved": "Настройки сериалов сохранены.",
  "settings.qualityProfileSection": "Профиль качества",
  "settings.qualityProfileCatalogLabel": "JSON-каталог профилей качества",
  "settings.qualityProfileCatalogHelp":
    "Определите поведение профилей в формате JSON. Сохраняйте только корректный JSON и список профилей с уникальными значениями `id`.",
  "settings.qualityProfileGlobalLabel":
    "Профиль качества по умолчанию (глобальный)",
  "settings.qualityProfileNameRequired":
    "Необходимо указать название профиля качества.",
  "settings.qualityProfileNameDuplicate":
    "Названия профилей качества должны быть уникальными.",
  "settings.qualityProfileGlobalHelp":
    "Используется, если для категории не задано собственное значение.",
  "settings.qualityProfileOverrideLabel":
    "Переопределение качества для категории {{category}}",
  "settings.qualityProfileOverridesLabel":
    "Переопределение профилей качества по категориям",
  "settings.qualityProfileOverrideHelp":
    "Используйте наследование, чтобы применять глобальный профиль по умолчанию для этой категории контента.",
  "settings.qualityProfileInheritLabel": "Наследовать глобальный профиль",
  "settings.qualityProfileCatalogInvalid":
    "Каталог профилей качества содержит некорректный JSON.",
  "settings.qualityProfileUnknown":
    'Неизвестный идентификатор профиля качества: "{{id}}".',
  "settings.qualitySettingsSaved": "Настройки качества сохранены.",
  "settings.downloadClientRoutingSaved":
    "Маршрутизация клиентов загрузки сохранена.",
  "settings.downloadClientRoutingSavedFor":
    'Маршрутизация клиентов загрузки сохранена для "{{name}}".',
  "settings.delayProfiles": "Профили задержки",
  "settings.acquisition": "Получение контента",
  "settings.acquisitionSaved": "Настройки получения контента сохранены.",
  "settings.acquisitionIntro":
    "RSS используется как основной механизм получения контента: для каждого требуемого элемента активный поиск выполняется один раз для каждого индексатора, после чего отслеживание продолжается через RSS. Активное пополнение выполняется постепенно и имеет ограниченный объём.",
  "settings.acquisitionEnabled": "Включить автоматическое получение контента",
  "settings.acquisitionEnabledHelp":
    "Запускает фоновый цикл получения контента: поиск для достижения актуального состояния и сопоставление через RSS.",
  "settings.acquisitionThresholds": "Пороги обновления",
  "settings.acquisitionSameTierMinDelta":
    "Минимальная разница оценки в одном уровне качества",
  "settings.acquisitionSameTierMinDeltaHelp":
    "Релиз того же уровня качества должен превзойти оценку текущего файла как минимум на указанное значение.",
  "settings.acquisitionConvergence": "Сходимость и распределение нагрузки",
  "settings.acquisitionPollIntervalSeconds": "Интервал опроса (секунды)",
  "settings.acquisitionPollIntervalSecondsHelp":
    "Как часто клиенты загрузки проверяются на наличие неудачных загрузок.",
  "settings.acquisitionWalkIntervalSeconds":
    "Интервал поискового обхода (секунды)",
  "settings.acquisitionWalkIntervalSecondsHelp":
    "Как часто библиотека проверяется на отсутствующие и улучшаемые медиафайлы и выполняется поиск для группы тайтлов. Добавление тайтла, изменение профиля качества и подобные изменения запускают обход сразу. Обход никогда не выполняется чаще интервала опроса.",
  "settings.acquisitionMaxScopesPerCycle": "Максимум элементов за цикл",
  "settings.acquisitionMaxScopesPerCycleHelp":
    "Максимальное количество элементов, проверяемых за один цикл. Фактическая частота запросов определяется ограничениями индексаторов, а не этим параметром.",
  "settings.acquisitionReconvergeDays": "Повторная проверка через (дней)",
  "settings.acquisitionReconvergeDaysHelp":
    "Значение 0 отключает функцию (рекомендуется): обработанные элементы продолжают отслеживаться через RSS. Включайте только в случае неполных RSS-лент.",
  "settings.delayProfileExisting": "Существующие профили задержки",
  "settings.delayProfileNone":
    "Профили задержки не настроены. Релизы будут загружаться немедленно.",
  "settings.delayProfileCreate": "Создать профиль",
  "settings.delayProfileCreateNew": "Создать новый профиль задержки",
  "settings.delayProfileEdit": "Изменить профиль",
  "settings.delayProfileConfirmDiscardTitle":
    "Отменить изменения текущего профиля задержки?",
  "settings.delayProfileConfirmDiscardDescription":
    "Все изменения текущего профиля задержки будут потеряны.",
  "settings.delayProfileDeleteConfirm": "Удалить этот профиль задержки?",
  "settings.delayProfileNameLabel": "Название",
  "settings.delayProfileNamePlaceholder":
    "Например: ожидание более качественного релиза",
  "settings.delayProfileNameRequired":
    "Необходимо указать название профиля задержки.",
  "settings.delayProfileUsenetDelay": "Задержка Usenet (минуты)",
  "settings.delayProfileUsenetDelayHelp":
    "Как долго удерживать релизы Usenet перед загрузкой. 0 — загрузка выполняется немедленно.",
  "settings.delayProfileTorrentDelay": "Задержка торрентов (минуты)",
  "settings.delayProfileTorrentDelayHelp":
    "Как долго удерживать торрент-релизы перед загрузкой. 0 — загрузка выполняется немедленно.",
  "settings.delayProfilePreferred": "Предпочтительный протокол",
  "settings.delayProfilePreferredHelp":
    "Обход ограничений по оценке применяется только к релизам, использующим предпочтительный протокол.",
  "settings.delayProfileProtocolModeLabel": "Режим протокола",
  "settings.delayProfileProtocolModeHelp":
    "Задержки отсчитываются с момента публикации релиза индексатором. Предпочтительный протокол выбирает между равнозначными релизами и определяет возможность обхода задержки.",
  "settings.delayProfileProtocolMode.preferUsenet": "Предпочитать Usenet",
  "settings.delayProfileProtocolMode.preferTorrent": "Предпочитать торрент",
  "settings.delayProfileProtocolMode.onlyUsenet": "Только Usenet",
  "settings.delayProfileProtocolMode.onlyTorrent": "Только торрент",
  "settings.delayProfileMinAge": "Минимальный возраст (минуты)",
  "settings.delayProfileMinAgeHelp":
    "Только для Usenet. Жёсткое ограничение: релизы моложе указанного значения будут удерживаться независимо от оценки. Это предотвращает загрузку незавершённых NZB-файлов. 0 — функция отключена.",
  "settings.delayProfileBypassLabel": "Порог оценки для обхода задержки",
  "settings.delayProfileBypassPlaceholder":
    "Оставьте пустым, чтобы не обходить задержку",
  "settings.delayProfileBypassHelp":
    "Если оценка релиза с предпочтительным протоколом равна или превышает этот порог, он будет загружен немедленно независимо от установленной задержки.",
  "settings.delayProfileBypassHighestQualityLabel":
    "Обходить задержку для наивысшего качества",
  "settings.delayProfileBypassHighestQualityHelp":
    "Немедленно загружать подходящий релиз наивысшего качества, если он использует предпочтительный протокол.",
  "settings.delayProfileFacetsLabel": "Применяется к",
  "settings.delayProfileFacetsHelp":
    "Типы контента, к которым применяется этот профиль. Оставьте все флажки снятыми, чтобы применять ко всем типам.",
  "settings.delayProfileLibrariesLabel": "Библиотеки",
  "settings.delayProfileLibrariesHelp":
    "Библиотеки, к которым применяется этот профиль. Оставьте выбранными все, чтобы применять его ко всем библиотекам. При удалении последней библиотеки профиля профиль отключается.",
  "settings.delayProfileUnknownLibrary": "Неизвестная библиотека",
  "settings.delayProfileAllFacets": "Все",
  "settings.delayProfileTagsLabel": "Теги",
  "settings.delayProfileTagsPlaceholder":
    "Теги через запятую (пусто = профиль по умолчанию для всех)",
  "settings.delayProfileTagsHelp":
    "Сопоставляет материалы по тегам. Если поле пустое, этот профиль используется по умолчанию для всех подходящих материалов.",
  "settings.delayProfilePriorityLabel": "Приоритет",
  "settings.delayProfilePriorityHelp":
    "Меньшее число = более высокий приоритет. Используется первый подходящий профиль.",
  "settings.delayProfileEnabledLabel": "Включено",
  "settings.delayProfilesSaved": "Профили задержки сохранены.",
  "settings.delayProfileSaveError": "Не удалось сохранить профили задержки.",
  "settings.mediaServersSection": "Подключения к медиасерверам",
  "settings.mediaServerCreate": "Сохранить медиасервер",
  "settings.mediaServerCreateNew": "Подключить новый медиасервер",
  "settings.mediaServerUpdate": "Обновить медиасервер",
  "settings.mediaServerConfirmDiscardTitle":
    "Отменить текущие изменения медиасервера?",
  "settings.mediaServerConfirmDiscardDescription":
    "Текущие изменения настроек медиасервера будут потеряны.",
  "settings.mediaServerEnabled": "Включить подключение к медиасерверу",
  "settings.mediaServerAuthCapabilities": "Разрешения",
  "settings.mediaServerAutoAddEnabled":
    "Автоматически добавлять новых подтверждённых пользователей",
  "settings.defaultAppPermissions": "Разрешения приложения по умолчанию",
  "settings.defaultLibraryPermissions": "Разрешения библиотеки по умолчанию",
  "settings.mediaServerPathMappings": "Сопоставление путей",
  "settings.mediaServerPathMappingsHelp":
    "Каждое правило сохраняется в формате: путь сервера => путь Scryer.",
  "settings.mediaServerValidation":
    "Необходимо указать название и базовый URL.",
  "settings.noMediaServersFound": "Подключения к медиасерверам не найдены.",
  "settings.capabilities": "Возможности",
  "settings.credentials": "Учётные данные",
  "settings.apiKeyConfigured": "API-ключ сохранён",
  "settings.apiKeyMissing": "API-ключ отсутствует",
  "settings.plexServerSelected": "Сервер Plex выбран",
  "settings.plexServerMissing": "Сервер Plex не выбран",
  "settings.discoverPlexServers": "Войдите в Plex для поиска серверов",
  "settings.plexServer": "Сервер Plex",
  "settings.plexServerDiscoveryEmpty":
    "Plex не вернул ни одного доступного сервера.",
  "settings.plexServerDiscoveryRequired":
    "Войдите в Plex и выберите сервер, прежде чем включать вход через Plex, привязку учётных записей или автоматическое добавление.",
  "settings.setupViaApiKey": "Настроить с помощью API-ключа",
  "settings.loginAsAdmin": "Войти как администратор",
  "settings.clearSavedApiKey": "Удалить сохранённый API-ключ",
  "settings.adminUsername": "Имя пользователя администратора",
  "settings.adminPassword": "Пароль администратора",
  "settings.plexMachineIdPlaceholder": "Plex machineIdentifier",
  "settings.mediaServerConnection": "Подключение к медиасерверу",
  "settings.manualConfiguration": "Ручная настройка",
  "settings.downloadClientSection": "Настройки клиента загрузки",
  "settings.downloadClientCreate": "Подключить клиент загрузки",
  "settings.downloadClientCreateNew": "Подключить клиент загрузки",
  "settings.downloadClientUpdate": "Обновить клиент загрузки",
  "settings.downloadClientConfirmDiscardTitle":
    "Отменить текущие изменения клиента загрузки?",
  "settings.downloadClientConfirmDiscardDescription":
    "Текущие изменения настроек клиента загрузки будут потеряны.",
  "settings.downloadClientUseSsl": "Использовать SSL",
  "settings.downloadClientUrlBase": "Базовый URL",
  "settings.downloadClientUrlPreview": "Рассчитанный базовый URL",
  "settings.downloadClientCategory": "Категория",
  "settings.downloadClientRecentPriority": "Приоритет новых загрузок в очереди",
  "settings.downloadClientOlderPriority": "Приоритет старых загрузок в очереди",
  "settings.downloadClientPriorityForce": "Принудительный",
  "settings.downloadClientPriorityVeryHigh": "Очень высокий",
  "settings.downloadClientPriorityHigh": "Высокий",
  "settings.downloadClientPriorityNormal": "Обычный",
  "settings.downloadClientPriorityLow": "Низкий",
  "settings.downloadClientPriorityVeryLow": "Очень низкий",
  "settings.downloadClientClientPriority": "Приоритет клиента",
  "settings.downloadClientPriority": "Приоритет",
  "settings.downloadClientRemoveCompleted": "Удалять завершённые",
  "settings.downloadClientRemoveFailed": "Удалять неудачные",
  "settings.downloadClientUseSslInfo":
    "Использовать SSL/TLS при подключении к этому клиенту загрузки.",
  "settings.downloadClientRemoveCompletedInfo":
    "Завершённые загрузки будут удаляться из истории клиента загрузки.",
  "settings.downloadClientRemoveFailedInfo":
    "Неудачные загрузки будут удаляться из истории клиента загрузки.",
  "settings.downloadClientEnabledInfo":
    "Если параметр отключён, этот клиент будет пропускаться при маршрутизации загрузок.",
  "settings.downloadClientEnabledLabel": "Включить клиент загрузки",
  "settings.downloadClientFilesystemPathMapping":
    "Сопоставление путей файловой системы (расширенные настройки)",
  "settings.downloadClientFilesystemPathMappingHelp":
    "Сопоставление удалённых путей требуется редко. Используйте его только в том случае, если Scryer и этот клиент загрузки используют одну и ту же папку загрузок, но видят её по разным путям файловой системы.",
  "settings.downloadClientRemotePathMappings": "Сопоставление удалённых путей",
  "settings.downloadClientRemotePathMappingsHelp":
    "Каждое правило сохраняется в формате УДАЛЁННЫЙ ПУТЬ => ЛОКАЛЬНЫЙ ПУТЬ.",
  "settings.downloadClientArtifactMode": "Режим обработки завершённых загрузок",
  "settings.downloadClientArtifactModeHelp":
    "Выберите, как Scryer должен обрабатывать завершённые загрузки, совместимые с SAB. Используйте NZBDAV .strm для подключённых артефактов .strm или NZBDAV Plex Symlink, если completed-symlinks подключён локально через rclone --links.",
  "settings.downloadClientArtifactModePlain":
    "Обычная загрузка в файловую систему",
  "settings.downloadClientArtifactModeNzbdavStrm": "NZBDAV .strm",
  "settings.downloadClientArtifactModeNzbdavSymlink":
    "Символическая ссылка NZBDAV Plex",
  "settings.downloadClientRemotePathMappingsRemoteLabel": "Удалённый путь",
  "settings.downloadClientRemotePathMappingsRemoteHelp":
    "Путь, сообщаемый клиентом загрузки.",
  "settings.downloadClientRemotePathMappingsLocalLabel": "Локальный путь",
  "settings.downloadClientRemotePathMappingsLocalHelp":
    "Путь, который Scryer должен использовать на этом компьютере.",
  "settings.downloadClientRemotePathMappingsAdd": "Добавить сопоставление",
  "settings.downloadClientRemotePathMappingsEmptyState":
    "Сопоставления путей ещё не настроены. Добавьте их только в том случае, если Scryer не может получить доступ к пути, сообщаемому клиентом загрузки.",
  "settings.downloadClientRemotePathMappingsExampleLabel": "Пример",
  "settings.downloadClientRemotePathMappingsRemoteRequired":
    "При наличии строки сопоставления необходимо указать удалённый путь.",
  "settings.downloadClientRemotePathMappingsLocalRequired":
    "При наличии строки сопоставления необходимо указать локальный путь.",
  "settings.downloadClientRemotePathMappingsLocalAbsolute":
    "Локальный путь должен быть абсолютным.",
  "settings.downloadClientRemotePathMappingsRemoteDuplicate":
    "Удалённый путь уже используется в другом сопоставлении этого клиента.",
  "settings.downloadClientSabnzbdAuthHelp":
    "SABnzbd поддерживает аутентификацию как по API-ключу, так и по имени пользователя и паролю. Для Decypharr обычно используются следующие параметры: хост — :8282, базовый URL — /sabnzbd, имя пользователя — ваш URL Arr, пароль — токен Arr.",
  "settings.downloadClientSabnzbdNzbdavHelp":
    "Для NzbDav направьте завершённые загрузки, совместимые с SAB, в подключённую папку completed-symlinks, убедитесь, что Scryer имеет к ней локальный доступ, и добавьте сопоставления удалённых путей, если NzbDav сообщает другой путь.",
  "settings.downloadClientQbittorrentDecypharrHelp":
    "Для qBittorrent с поддержкой Decypharr используйте URL вашего Arr в качестве имени пользователя, а токен Arr — в качестве пароля. Затем укажите соответствующее имя Arr (например, radarr или sonarr) в разделе маршрутизации клиентов загрузки.",
  "settings.downloadClientDecypharrFilesystemHelp":
    "Импорт завершённых загрузок работает только с путями, к которым Scryer имеет локальный доступ. В случае Decypharr и NzbDav Scryer импортирует данные из папки завершённых символических ссылок, указанной клиентом. Поэтому подключите этот путь к Scryer и настройте сопоставление удалённых путей, если указанные пути отличаются.",
  "settings.downloadClientNamePlaceholder": "Мой клиент загрузки",
  "settings.downloadClientHostPlaceholder": "download.example.com",
  "settings.downloadClientUrlBasePlaceholder": "например, api",
  "settings.downloadClientPortPlaceholder": "8080",
  "settings.downloadClientPriorityPlaceholder": "По умолчанию",
  "settings.downloadClientCategoryPlaceholder": "series",
  "settings.downloadClientStatus": "Статус",
  "settings.downloadClientLastError": "Ошибка",
  "settings.downloadClientLastSeen": "Последняя активность: {{time}}",
  "settings.downloadClientNoActivity": "Нет активности",
  "settings.existingDownloadClients": "Существующие клиенты загрузки",
  "settings.downloadClientRoutingScope":
    "Метаданные клиента загрузки для {{scope}}",
  "settings.downloadClientRoutingGloballyEnabled": "Глобально включено",
  "settings.downloadClientRoutingEnabled": "Включено",
  "settings.downloadClientRoutingNoNzbClients":
    "Клиенты NZBGet не настроены. Добавьте хотя бы один клиент в разделе «Клиенты загрузки», чтобы настроить маршрутизацию медиакатегорий.",
  "settings.noDownloadClientsFound": "Клиенты загрузки не найдены.",
  "settings.indexerRoutingScope": "Метаданные индексатора для {{scope}}",
  "settings.indexerRoutingPriority": "Приоритет",
  "settings.indexerRoutingCategories": "Категории",
  "settings.indexerRoutingGloballyEnabled": "Глобально включено",
  "settings.indexerRoutingEnabled": "Включено",
  "settings.indexerRoutingNoCategories": "Категории не выбраны.",
  "settings.indexerRoutingCategoriesPlaceholder": "Выберите категории",
  "settings.indexerRoutingNoIndexers": "Индексаторы не найдены.",
  "settings.indexerRoutingCustomCategories": "Свои коды",
  "settings.indexerRoutingCustomCategoryPlaceholder":
    "Числовой id из caps индексатора",
  "settings.indexerRoutingCustomCategoryAdd": "Добавить",
  "settings.indexerRoutingIndexerCategories": "Также на этом индексаторе",
  "settings.indexerCategorySeries": "Сериалы",
  "settings.indexerCategoryMovies": "Фильмы",
  "settings.indexerCategoryOther": "Прочее",
  "settings.indexerCategoryForeign": "Иностранные",
  "settings.indexerCategorySd": "SD",
  "settings.indexerCategoryHd": "HD",
  "settings.indexerCategoryUhd": "UHD",
  "settings.indexerCategorySport": "Спорт",
  "settings.indexerCategoryAnime": "Аниме",
  "settings.indexerCategoryDocumentary": "Документальные",
  "settings.indexerCategoryBluRay": "Blu-ray",
  "settings.indexerCategoryThreeD": "3D",
  "settings.indexerCategoryMisc": "Разное",
  "indexerCategory.labelCategory": "Категория",
  "settings.downloadClientValidation":
    "Необходимо указать название и адрес хоста.",
  "settings.downloadClientBaseUrlRequired":
    "Необходимо указать корректные адрес хоста и порт.",
  "settings.qualitySource": "Источник",
  "settings.renameSection": "Переименование",
  "settings.folderTemplateLabel": "Шаблон папки",
  "settings.folderTemplatePlaceholder": "Заполнитель",
  "settings.folderTemplateRequired": "Необходимо указать шаблон папки.",
  "settings.seasonFolderTemplateLabel": "Шаблон папки сезона",
  "settings.seasonFolderTemplatePlaceholder": "Сезон {season}",
  "settings.seasonFolderTemplateRequired":
    "Необходимо указать шаблон папки сезона.",
  "settings.specialsFolderTemplateLabel": "Шаблон папки спецвыпусков",
  "settings.specialsFolderTemplatePlaceholder": "Спецвыпуски",
  "settings.specialsFolderTemplateRequired":
    "Необходимо указать шаблон папки спецвыпусков.",
  "settings.seasonFolderTemplateMustContainSeason":
    "Шаблон папки сезона должен содержать {season}.",
  "settings.seasonFolderValidationUnknownToken":
    "Шаблоны папок сезонов поддерживают токены {season}, {title}, {year} и идентификаторы внешних сервисов. Недопустимый токен: {{token}}",
  "settings.folderAvailableTokens": "Доступные токены папок",
  "settings.folderRenameSectionTitle": "Переименование папок",
  "settings.folderValidationUnknownToken":
    "Шаблоны папок поддерживают токены {title}, {year} и идентификаторы внешних сервисов. Недопустимый токен: {{token}}",
  "settings.folderValidationEmpty": "Шаблон папки не может быть пустым.",
  "settings.folderValidationLeafTitleToken":
    "Последняя папка в шаблоне должна содержать токен названия или внешнего идентификатора",
  "settings.folderValidationIllegalCharacter":
    "Шаблон папки содержит недопустимый символ файловой системы: {{character}}",
  "settings.renameTemplateLabel": "Шаблон переименования",
  "settings.renameTemplatePlaceholder": "Заполнитель",
  "settings.renameTemplateHelp":
    "Токены шаблона основаны на метаданных (например: {title}, {year}, {quality}, {ext}).",
  "settings.renameTemplateRequired":
    "Необходимо указать шаблон переименования.",
  "settings.renameCollisionPolicyLabel": "Политика обработки конфликтов",
  "settings.renameCollisionPolicySkip": "Пропустить",
  "settings.renameCollisionPolicyError": "Ошибка",
  "settings.renameCollisionPolicyReplaceIfBetter":
    "Заменить, если версия лучше",
  "settings.renameMissingMetadataPolicyLabel":
    "Политика отсутствующих метаданных",
  "settings.renameMissingMetadataPolicyFallbackTitle": "Использовать название",
  "settings.renameMissingMetadataPolicySkip": "Пропустить",
  "settings.renameSectionTitle": "Переименование файлов",
  "settings.renameReferenceTitle": "Справка",
  "settings.renameReferenceTokens": "Токены",
  "settings.renameReferenceFunctions": "Функции",
  "settings.renameReferenceFiles": "Файлы",
  "settings.renameReferenceFolders": "Папки",
  "settings.renameAvailableTokens": "Доступные токены",
  "settings.renameLiteralBracesLabel": "Литеральные фигурные скобки",
  "settings.renameSpaceFilterLabel": "Заменять пробелы",
  "settings.renameFunctionTruncateTitle": "Длина",
  "settings.renameFunctionWhitespaceTitle": "Пробелы",
  "settings.renameFunctionWhitespaceDescription":
    "Используйте один фильтр пробелов, чтобы выбрать разделитель для текста токена.",
  "settings.renameFunctionPaddingTitle": "Дополнение числами",
  "settings.renameFunctionChainTitle": "Цепочки",
  "settings.renameFunctionBracesTitle": "Фигурные скобки",
  "settings.renameTruncateFilterLabel": "Ограничить токен первыми N символами.",
  "settings.renameNumberPaddingLabel": "Дополнить числовые токены до N цифр.",
  "settings.renameSpaceUnderscoreFilterLabel":
    "Заменить пробелы символами подчёркивания.",
  "settings.renameSpaceDotFilterLabel": "Заменить пробелы точками.",
  "settings.renameSpaceDashFilterLabel": "Заменить пробелы дефисами.",
  "settings.renameSpaceRemoveFilterLabel":
    "Удалить пробелы из значения токена.",
  "settings.renameFilterChainLabel": "Применять фильтры слева направо.",
  "settings.renameTokenTitle": "Название",
  "settings.renameTokenYear": "Год выпуска",
  "settings.renameTokenTitleWithYear":
    "Название с годом; год добавляется, только если название ещё не заканчивается им",
  "settings.renameTokenTitleWithoutYear": "Название без года в конце",
  "settings.renameTokenQuality": "Качество (например, 1080p)",
  "settings.renameTokenEdition": "Издание (например, режиссёрская версия)",
  "settings.renameTokenImdbId": "Идентификатор IMDb",
  "settings.renameTokenTmdbId": "Идентификатор TMDb",
  "settings.renameTokenTvdbId": "Идентификатор TVDb",
  "settings.renameTokenAnidbId": "Идентификатор AniDB",
  "settings.renameTokenMalId": "Идентификатор MAL",
  "settings.renameTokenAnilistId": "Идентификатор AniList",
  "settings.renameTokenSource": "Источник (например, BluRay, WEB-DL)",
  "settings.renameTokenDynamicRange":
    "Динамический диапазон (например, HDR10, DV, HDR10+)",
  "settings.renameTokenRevision": "Ревизия релиза (Proper/REPACK)",
  "settings.renameTokenIs3d": "Обозначение 3D",
  "settings.renameTokenVideoCodec": "Видеокодек (например, x265)",
  "settings.renameTokenAudioCodec": "Аудиокодек (например, DTS)",
  "settings.renameTokenAudioChannels": "Аудиоканалы (например, 5.1)",
  "settings.renameTokenSeason": "Номер сезона",
  "settings.renameTokenSeasonOrder": "Порядок сезона (позиция в сюжете)",
  "settings.renameTokenEpisode": "Номер эпизода",
  "settings.renameTokenAbsoluteEpisode": "Абсолютный номер эпизода",
  "settings.renameTokenEpisodeTitle": "Название эпизода",
  "settings.renameTokenGroup": "Группа релиза",
  "settings.renameTokenExt": "Расширение файла",
  "settings.renameValidationUnmatchedOpen": "Незакрытая фигурная скобка '{'",
  "settings.renameValidationUnmatchedClose":
    "Лишняя закрывающая фигурная скобка '}'",
  "settings.renameValidationUnknownToken": "Неизвестный токен: {{token}}",
  "settings.renameValidationInvalidPadding":
    "Числовое дополнение должно быть целым числом ASCII в диапазоне от 0 до 240: {{padding}}",
  "settings.renameValidationInvalidFilter":
    "Недопустимый фильтр токена: {{filter}}",
  "settings.renameValidationEmpty":
    "Шаблон переименования не может быть пустым.",
  "settings.renameComingSoon":
    "Правила переименования появятся в ближайших версиях.",
  "settings.mediaSettingsSaved": "Настройки медиаконтента сохранены.",
  "rename.previewButton": "Предварительный просмотр",
  "rename.previewing": "Подготовка предпросмотра…",
  "rename.applyButton": "Применить переименование",
  "rename.applying": "Применение…",
  "rename.currentPath": "Текущий",
  "rename.proposedPath": "Предлагаемый",
  "rename.planSummary":
    "{{total}} элементов, {{renamable}} можно переименовать, {{noop}} без изменений, {{conflicts}} конфликтов, {{errors}} ошибок",
  "settings.title": "Настройки",
  "settings.sectionTitle": "Настройки • {{section}}",
  "settings.generalText": "Общие настройки приложения.",
  "settings.verificationDepthTitle": "Проверка копий при импорте",
  "settings.verificationDepthHelp":
    "Насколько тщательно Scryer проверяет скопированный файл, прежде чем трогать оригинал.",
  "settings.verificationDepthScope":
    "Относится только к копиям, создаваемым при импорте из клиентов загрузки. Перемещения библиотек и корней всегда проверяются полностью, независимо от этой настройки.",
  "settings.verificationDepthLabel": "Глубина проверки",
  "settings.verificationDepthFull": "Полное перечитывание",
  "settings.verificationDepthFullHelp":
    "Полностью перечитывает копию и сравнивает её с контрольной суммой, вычисленной во время копирования.",
  "settings.verificationDepthQuick": "Быстрая проверка (по умолчанию)",
  "settings.verificationDepthQuickHelp":
    "Сравнивает размер файла и выборку начала и конца. Быстрее, но гарантия слабее.",
  "settings.verificationDepthSaved": "Глубина проверки сохранена.",
  "settings.generalPlaceholder":
    "Настройка языка и других глобальных параметров.",
  "settings.dateTimeFormatLabel": "Формат даты и времени",
  "settings.dateTimeFormatLocale": "Региональный формат по умолчанию",
  "settings.dateTimeFormatIso24h": "ГГГГ-ММ-ДД, 24-часовой формат",
  "settings.dateTimeFormatHelp":
    "Определяет формат отображения даты и времени в интерфейсе.",
  "settings.historyRetentionTitle": "Хранение истории",
  "settings.historyRetentionHelp":
    "Этот параметр определяет срок хранения истории пользователей и журнала аудита Scryer. Изменения вступят в силу при следующем запуске процедуры очистки.",
  "settings.historyRetentionExternalHelp":
    "Очистка временных данных, связанных с получением контента, всё равно будет выполняться. Этот параметр не влияет на историю, сохраняемую SABnzbd, NZBGet или Weaver.",
  "settings.featuresHeader": "Функции",
  "settings.experimentalFeaturesLabel": "Включить экспериментальные функции",
  "settings.experimentalFeaturesHelp":
    "Показывает функции, которые ещё дорабатываются для всех на этом экземпляре: перемещение тайтлов между библиотеками и корневыми каталогами, а также правила обслуживания.",
  "settings.personalizedDiscoveryLabel": "Персонализированные рекомендации",
  "settings.personalizedDiscoveryHelp":
    "Отправляет сводку вашей библиотеки в шлюз метаданных, чтобы рекомендации могли предлагать тайтлы на основе того, что у вас есть. Отключите, чтобы ограничить рекомендации только публичными лентами.",
  "settings.srrdbFilenameRecoveryLabel":
    "Восстанавливать запутанные имена файлов через srrdb.com",
  "settings.srrdbFilenameRecoveryHelp":
    "Когда автоматический импорт из SABnzbd или NZBGet содержит видеофайлы с запутанными именами, Scryer отправляет контрольную сумму CRC-32 и размер каждого файла в стороннюю службу srrdb.com, чтобы найти исходное имя файла. Больше ничего о вашей библиотеке или файле не отправляется. Файлы на диске никогда не переименовываются.",
  "discovery.personalizedDisabled":
    "Персонализированные рекомендации отключены для этого экземпляра.",
  "settings.keepHistoryForever": "Хранить историю бессрочно",
  "settings.historyRetentionDaysHeader": "Хранить историю",
  "settings.historyRetentionDaysSuffix": "дней",
  "settings.securitySessionDurationSuffix": "дней",
  "settings.historyRetentionDaysLabel": "Хранить историю в течение N дней",
  "settings.historyRetentionSaved": "Настройки хранения истории сохранены.",
  "settings.generalSaved": "Общие настройки сохранены.",
  "settings.uiSaved": "Настройки отображения сохранены.",
  "settings.historyRetentionValidation":
    "Срок хранения истории должен составлять не менее 1 дня, если не выбрано бессрочное хранение.",
  "settings.imageCacheTitle": "Кэш изображений",
  "settings.imageCacheMaxSizeLabel": "Размер кэша изображений",
  "settings.imageCacheMbSuffix": "МиБ",
  "settings.imageCacheEnvOverride":
    "Параметр SCRYER_IMAGE_CACHE_MAX_BYTES активен. Сейчас используется его значение; сохранённое ограничение начнёт действовать после удаления переопределения из переменных окружения.",
  "settings.imageCacheMaxSizeValidation":
    "Максимальный размер кэша изображений должен составлять не менее 1 МиБ.",
  "settings.clearImageCache": "Очистить кэш изображений",
  "settings.clearingImageCache": "Очистка кэша изображений…",
  "settings.imageCacheClearQueued":
    "Очистка кэша изображений поставлена в очередь.",
  "settings.pluginHttpTrustTitle": "Доверенные HTTPS-сертификаты плагинов",
  "settings.pluginHttpTrustAdvancedLabel": "Дополнительная функция",
  "settings.pluginHttpTrustStoredCount": "Сохранено: {{count}}",
  "settings.pluginHttpTrustDescription":
    "Расширяет список доверенных HTTPS-сертификатов для исходящих запросов WASM-плагинов за счёт дополнительных частных или самостоятельно управляемых центров сертификации.",
  "settings.pluginHttpTrustWarningTitle":
    "Используйте эту функцию только в том случае, если вы доверяете центру сертификации.",
  "settings.pluginHttpTrustWarningBody":
    "Эти сертификаты расширяют доверие к HTTPS-соединениям для исходящих запросов плагинов. Загружайте только сертификаты центров сертификации, которыми вы управляете или которым полностью доверяете.",
  "settings.pluginHttpTrustUploadButton": "Загрузить доверенные сертификаты",
  "settings.pluginHttpTrustClearButton": "Удалить все",
  "settings.pluginHttpTrustEmpty":
    "Дополнительные доверенные сертификаты не загружены.",
  "settings.pluginHttpTrustEntryLabel": "Доверенный сертификат",
  "settings.pluginHttpTrustUploadMissingCertificate":
    "Загруженный файл не содержит сертификат в формате PEM или DER.",
  "settings.pluginHttpTrustUploadTrailingText":
    "Загруженный PEM-файл содержит неожиданный текст, не относящийся к сертификату.",
  "settings.pluginHttpTrustUploadInvalidCertificate":
    "Не удалось обработать загруженный сертификат.",
  "settings.pluginHttpTrustUploadReadError":
    "Не удалось обработать загруженный файл сертификата.",
  "settings.tlsTitle": "TLS / SSL",
  "settings.tlsCertPathLabel": "Путь к TLS-сертификату",
  "settings.tlsCertPathHelp":
    "Абсолютный путь к файлу сертификата в формате PEM. Для применения изменений требуется перезапуск.",
  "settings.tlsCertPathPlaceholder": "/etc/scryer/cert.pem",
  "settings.tlsKeyPathLabel": "Путь к закрытому TLS-ключу",
  "settings.tlsKeyPathHelp":
    "Абсолютный путь к файлу закрытого ключа в формате PEM. Для применения изменений требуется перезапуск.",
  "settings.tlsKeyPathPlaceholder": "/etc/scryer/key.pem",
  "settings.tlsSaved":
    "Настройки TLS сохранены. Перезапустите службу, чтобы изменения вступили в силу.",
  "settings.languageChangeTitle": "Изменение языка",
  "settings.languageChangeWarning":
    "После переключения на язык {{language}} будут повторно загружены метаданные для всех материалов в вашей библиотеке (названия, описания и заголовки для сортировки). Интерфейс изменится сразу, но обновление всего содержимого может занять до 24 часов.",
  "settings.languageChangeConfirm": "Изменить язык",
  "settings.metadataRehydrationStarted":
    "Обновление метаданных запущено. Материалы будут обновляться в фоновом режиме.",
  "settings.tlsRestartNote":
    "Изменения параметров TLS требуют перезапуска службы.",
  "theme.light": "Светлая",
  "theme.dark": "Тёмная",
  "theme.system": "Системная",
  "theme.switchLabel": "Сменить тему (текущая: {{theme}})",
  "search.globalPlaceholder":
    "Поиск фильмов, сериалов, аниме, действий и настроек…",
  "search.globalPlaceholderNoLibrary":
    "Поиск фильмов, сериалов, аниме, действий и настроек…",
  "search.overlayPlaceholder":
    "Поиск по библиотекам, действиям, настройкам и другим разделам…",
  "search.overlayPlaceholderNoLibrary":
    "Поиск фильмов, сериалов, аниме, действий и настроек…",
  "search.shortcutHint": "⌘K",
  "search.shortcutHintControl": "Ctrl+K",
  "search.title": "Глобальный поиск",
  "search.subtitle":
    "Поиск фильмов, сериалов, аниме, элементов библиотеки, действий и настроек.",
  "search.subtitleNoLibrary":
    "Поиск фильмов, сериалов, аниме, действий и настроек.",
  "search.subtitleWithQuery": "Результаты поиска по запросу «{{query}}»",
  "search.statusLoading": "Поиск по запросу «{{query}}»…",
  "search.statusResultOne": "1 результат по запросу «{{query}}»",
  "search.statusResultOther":
    "Найдено результатов: {{count}} по запросу «{{query}}»",
  "search.statusNoResults": "По запросу «{{query}}» ничего не найдено",
  "search.tabAll": "Все",
  "search.tabLibrary": "В библиотеке",
  "search.tabActions": "Действия",
  "search.catalog": "Каталог",
  "search.inLibrary": "В библиотеке",
  "search.goToHint": "навигация и действия",
  "search.actionsAndSettings": "Действия и настройки",
  "search.alreadyInCollection": "уже находится в вашей коллекции",
  "search.everywhereElse": "Другие разделы",
  "search.resultCountOne": "1 результат",
  "search.resultCountOther": "{{count}} результатов",
  "search.monitored": "Отслеживается",
  "search.unmonitored": "Не отслеживается",
  "search.view": "Открыть",
  "search.viewAll": "Показать всё",
  "search.viewAll.movie": "Показать все фильмы",
  "search.viewAll.series": "Показать все сериалы",
  "search.viewAll.anime": "Показать всё аниме",
  "search.add": "Добавить",
  "search.addToAnotherLibrary": "Добавить в другую библиотеку",
  "search.requestForAnotherLibrary": "Запросить для другой библиотеки",
  "search.chooseLibraryToView":
    "Выберите библиотеку, в которой открыть {{name}}",
  "search.footerTip":
    "Не удалось найти нужный результат? Попробуйте другой запрос или проверьте настройки индексаторов.",
  "search.searchTips": "Советы по поиску",
  "search.tipTitles":
    "Выполняйте поиск по названию фильма, сериала, аниме, каталога, действия или настройки.",
  "search.tipTitlesNoLibrary":
    "Выполняйте поиск по названию фильма, сериала, аниме, действия или настройки.",
  "search.tipTabs":
    "Используйте вкладки, чтобы ограничить результаты библиотекой или определённым типом каталога.",
  "search.tipTabsNoLibrary":
    "Используйте вкладки, чтобы ограничить результаты определённым типом каталога.",
  "search.tipIndexers":
    "Используйте более общий запрос, прежде чем проверять индексаторы для материалов, отсутствующих в результатах поиска по метаданным.",
  "search.noMatchesFor": "По запросу «{{query}}» ничего не найдено",
  "search.emptyHint":
    "Попробуйте выполнить поиск по другому названию фильма, сериала, аниме, каталога, действия или настройки.",
  "search.emptyHintNoLibrary":
    "Попробуйте выполнить поиск по другому названию фильма, сериала, аниме, действия или настройки.",
  "search.minimumQueryTitle": "Продолжайте ввод",
  "search.minimumQueryHint":
    "Введите не менее 2 символов для поиска по библиотеке и метаданным.",
  "search.minimumQueryHintNoLibrary":
    "Введите не менее 2 символов для поиска по метаданным.",
  "search.noCatalogMatches": "Совпадений в управляемых каталогах пока нет.",
  "search.noMetadataMatches":
    "В этом разделе совпадений по метаданным пока нет.",
  "search.alreadyCataloged": "Уже добавлено в каталог",
  "search.cataloged": "Добавлено в каталог",
  "search.unavailable": "Недоступно",
  "search.configureAdd": "Настроить параметры добавления",
  "search.addConfigLibrary": "Библиотека",
  "search.addConfigQualityProfile": "Профиль качества",
  "search.addConfigInheritLibrary": "Наследовать настройки библиотеки",
  "search.addConfigRootFolder": "Корневая папка",
  "search.addConfigSeasonFolder": "Папка сезона",
  "search.addConfigMonitorType": "Тип отслеживания",
  "search.addConfigSelectQualityProfile": "Выберите профиль качества",
  "search.addConfigNoQualityProfiles": "Профили качества не настроены.",
  "search.seasonFolder.enabled": "Включено",
  "search.seasonFolder.disabled": "Отключено",
  "search.monitorType.monitored": "Отслеживается",
  "search.monitorType.unmonitored": "Не отслеживается",
  "search.monitorType.futureEpisodes": "Будущие эпизоды",
  "search.monitorType.missingAndFutureEpisodes":
    "Отсутствующие и будущие эпизоды",
  "search.monitorType.allEpisodes": "Все эпизоды",
  "search.monitorType.none": "Нет",
  "search.adding": "Добавление…",
  "search.request": "Запрос",
  "search.requesting": "Выполнение запроса…",
  "search.facetMovie": "Фильм",
  "search.facetSeries": "Сериал",
  "search.facetAnime": "Аниме",
  "search.metadataSearch": "Поиск по метаданным",
  "label.back": "Назад",
  "label.close": "Закрыть",
  "label.dismiss": "Скрыть",
  "pwa.offline": "Вы не в сети. Некоторые функции могут быть недоступны.",
  "pwa.installApp": "Установить Scryer",
  "pwa.iosInstallHint":
    "Нажмите кнопку «Поделиться» и выберите «Добавить на экран «Домой»», чтобы установить приложение.",
  "smgUpgrade.deprecatedTitle": "Рекомендуется обновить Scryer",
  "smgUpgrade.deprecatedBody":
    "Поддержка этой версии Scryer скоро завершится. Обновите приложение, чтобы продолжить работу.",
  "smgUpgrade.blockedTitle": "Требуется обновление Scryer",
  "smgUpgrade.blockedBody":
    "Эта версия Scryer больше не поддерживается. Обновите приложение, чтобы продолжить работу.",
  "smgUpgrade.minimumVersion": "Минимальная поддерживаемая версия: {{version}}",
  "smgUpgrade.deadline": "Окончание поддержки: {{date}}",
  "smgUpdate.title": "Доступно обновление Scryer",
  "smgUpdate.body":
    "Вы используете версию {{current}}. Последняя версия: {{latest}}.",
  "smgUpdate.releaseNotes": "Примечания к выпуску",
  "title.addAndQueue": "Добавить и поставить в очередь",
  "title.addToCatalog": "Добавить в каталог",
  "title.name": "Название",
  "title.namePlaceholder": "Поиск в TVDB по названию",
  "title.facet": "Категория",
  "title.manageMovies": "Управление фильмами",
  "title.manageSeries": "Управление сериалами",
  "title.manageAnime": "Управление аниме",
  "title.monitored": "Отслеживается",
  "title.unmonitored": "Не отслеживается",
  "title.ended": "Завершено",
  "title.continuing": "Продолжается",
  "title.upcoming": "Скоро",
  "title.backToFacet": "Назад к разделу «{{facet}}»",
  "title.notFound": "Материал не найден.",
  "title.noPoster": "Нет постера",
  "title.addedAt": "Добавлено {{date}}",
  "title.monitorAction": "Отслеживать",
  "title.unmonitorAction": "Прекратить отслеживание",
  "title.markWantedAction": "Отметить как желаемое",
  "title.searchMonitoredAction": "Искать отслеживаемые",
  "title.refreshAndScanAction": "Обновить и просканировать",
  "title.filterPlaceholder": "Фильтр управляемых материалов",
  "title.loading": "Данные не загружены",
  "title.noManaged": "В этом разделе материалы не найдены.",
  "title.statusTemplate": "Загружено материалов: {{count}}.",
  "title.contextPanelTitle": "Сведения о материале",
  "title.contextWorkspaceTitle": "Рабочая область материала",
  "title.contextTitleList": "Список материалов",
  "title.contextListDisclosure": "Список",
  "title.showTitleList": "Показать список",
  "title.hideTitleList": "Скрыть список",
  "title.contextEmptyTitle": "Материал не выбран",
  "title.contextEmptyBody": "Нет данных для предварительного просмотра.",
  "title.contextForYouTitle": "Рекомендации",
  "title.contextForYouSubtitle": "Рекомендации на основе вашей библиотеки",
  "title.contextForYouBody":
    "Откройте рабочую область материала или изучите рекомендации, подобранные на основе вашей коллекции.",
  "title.contextForYouTop": "Лучшее",
  "title.contextForYouTopMoviesThisWeek": "Лучшие фильмы недели",
  "title.contextForYouTopSeriesThisWeek": "Лучшие сериалы недели",
  "title.contextForYouTopAnimeThisWeek": "Лучшее аниме недели",
  "title.contextForYouGenre": "Потому что вам нравится жанр {{genre}}",
  "title.contextForYouTag": "Потому что вам нравится тег {{tag}}",
  "title.contextForYouAcclaimed": "Высоко оценённые — отсутствуют в библиотеке",
  "title.contextForYouCompleteCollection": "Дополните коллекцию",
  "title.contextForYouRecent": "Недавно добавленные",
  "title.contextForYouReasonTop": "Максимальное соответствие вашей библиотеке",
  "title.contextForYouReasonWeekly":
    "Популярно на этой неделе в вашей библиотеке",
  "title.contextForYouReasonGenre": "Соответствует вашей подборке «{{genre}}»",
  "title.contextForYouReasonTag": "Соответствует вашей подборке «{{tag}}»",
  "title.contextForYouReasonAcclaimed":
    "Высокий рейтинг и отсутствует в вашей библиотеке",
  "title.contextForYouReasonCollection":
    "Входит в коллекцию, которая у вас уже есть",
  "title.contextForYouReasonRecent": "Недавно добавлено в вашу библиотеку",
  "title.contextForYouMatch": "Совпадение: {{match}}",
  "title.contextForYouEmptyTitle": "Рекомендаций пока нет",
  "title.contextForYouEmptyBody":
    "Добавьте материалы или просканируйте библиотеку, чтобы сформировать рекомендации.",
  "title.contextMoreLikeThis": "Похожие материалы",
  "title.contextMoreLikeThisScope": "Из текущего раздела",
  "title.contextBlockedReleases": "Заблокированные релизы",
  "title.contextBlockedReleasesEmpty":
    "Для этого материала заблокированные релизы отсутствуют.",
  "title.contextOverview": "Обзор",
  "title.contextLibrary": "Библиотека",
  "title.contextQuality": "Качество",
  "title.contextRootFolder": "Корневая папка",
  "title.contextSize": "Размер",
  "title.contextEpisodes": "Эпизоды",
  "title.contextAdded": "Добавлено",
  "title.contextStorage": "Хранилище",
  "title.contextPrimary": "Основное",
  "title.contextMetadata": "Метаданные",
  "title.contextMonitoring": "Мониторинг",
  "title.contextStatus": "Статус",
  "title.contextRuntime": "Продолжительность",
  "title.contextStudio": "Студия",
  "title.contextNetwork": "Телеканал",
  "title.contextLanguage": "Язык",
  "title.contextCountry": "Страна",
  "title.contextReleased": "Дата выхода",
  "title.contextFirstAired": "Первая трансляция",
  "title.contextMetadataFetched": "Метаданные получены",
  "title.contextSortTitle": "Название для сортировки",
  "title.contextMonitorType": "Тип мониторинга",
  "title.contextSeasonFolders": "Папки сезонов",
  "title.contextSpecials": "Спецвыпуски",
  "title.contextFiller": "Филлеры",
  "title.contextRecaps": "Краткие пересказы",
  "title.contextInterSeasonMovies": "Фильмы между сезонами",
  "title.openOverview": "Открыть обзор",
  "title.columns": "Столбцы",
  "title.toggleColumns": "Настроить столбцы",
  "title.table.groupCore": "Основное",
  "title.table.groupRatings": "Рейтинги",
  "title.table.groupMedia": "Медиа",
  "title.table.groupOperational": "Система",
  "title.table.facet": "Категория",
  "title.table.library": "Библиотека",
  "title.table.monitored": "Отслеживается",
  "title.table.poster": "Постер",
  "title.table.qualityTier": "Качество",
  "title.table.profile": "Профиль",
  "title.table.episodes": "Эпизоды",
  "title.table.year": "Год",
  "title.table.runtime": "Продолжительность",
  "title.table.avgRuntime": "Средняя продолжительность",
  "title.table.episodeProgressTooltip": "{{owned}} / {{total}} эпизодов",
  "title.table.episodeProgressTooltipWithTotal":
    "{{owned}} / {{total}} эпизодов (отслеживается: {{monitored}})",
  "title.table.status": "Статус",
  "title.table.root": "Корневая папка",
  "title.table.popularity": "Популярность",
  "title.table.resolution": "Разрешение",
  "title.table.hdr": "HDR",
  "title.table.audioCodec": "Аудиокодек",
  "title.table.size": "Размер",
  "title.viewModeToggle": "Режим отображения",
  "title.viewModeCompact": "Компактный",
  "title.viewModePosterTable": "Список",
  "title.viewModeTable": "Таблица",
  "title.viewModePoster": "Постеры",
  "title.bulkSelectionCount": "Выбрано: {{count}}",
  "title.bulkSelectionPanelTitle": "Множественный выбор",
  "title.bulkSelectionPanelHint":
    "Выберите действие для отмеченных материалов или очистите выделение.",
  "title.bulkEditTitle": "Изменить выбранные материалы",
  "title.bulkEditDescription":
    "Применить изменения к {{count}} выбранным материалам.",
  "title.editOptionsTitle": "Изменение параметров",
  "title.editOptionsDescription": "Изменить параметры для {{name}}.",
  "title.bulkDeleteTitle": "Удалить выбранные материалы",
  "title.bulkDeleteDescription":
    "Удалить из каталога {{count}} выбранных материалов.",
  "title.selectTitle": "Выбрать {{name}}",
  "title.selectAllTitles": "Выбрать все отображаемые материалы",
  "title.interactiveSearchComingSoon":
    "Интерактивный поиск появится в ближайшее время.",
  "title.seasonMonitored": "Сезон отслеживается",
  "title.episodeMonitored": "Эпизод отслеживается",
  "title.seriesMovieMonitored": "Фильм из сериала отслеживается",
  "title.deleteFilesOnDisk": "Безвозвратно удалить файлы с диска",
  "title.seasonsAndEpisodes": "Сезоны и эпизоды",
  "title.noTrackedSeasons":
    "Для этого сериала пока не отслеживается ни один сезон.",
  "title.fetchingData": "Получение данных",
  "title.filesOnDisk": "Файлы на диске",
  "title.searchSubtitles": "Поиск субтитров",
  "title.noFilesTracked": "Файлы не отслеживаются.",
  "title.noFilesTrackedHint":
    "Запустите сканирование библиотеки, чтобы обнаружить файлы на диске.",
  "title.configureRootFoldersHint":
    "Настройте корневые папки данных в параметрах категории перед сканированием этой библиотеки.",
  "title.invalidRootFoldersTitle":
    "Одна или несколько корневых папок недействительны.",
  "title.invalidRootFoldersHint":
    "Исправьте настроенные корневые папки данных в параметрах категории перед запуском сканирования библиотеки.",
  "title.configureRootFoldersButton": "Настроить корневые папки данных",
  "title.descriptionUnavailable": "Описание отсутствует.",
  "title.canon": "Канон",
  "title.mixed": "Смешанный",
  "title.specials": "Спецвыпуски",
  "title.seasonNumber": "Сезон {{number}}",
  "title.episodeRange": "Эпизоды {{start}}–{{end}}",
  "title.episodeCountOne": "{{count}} эпизод",
  "title.episodeCountOther": "{{count}} эпизодов",
  "title.contextMoreEpisodesOne": "Ещё {{count}} эпизод",
  "title.contextMoreEpisodesOther": "Ещё {{count}} эпизодов",
  "title.contextMoreSeasonsOne": "Ещё {{count}} сезон",
  "title.contextMoreSeasonsOther": "Ещё {{count}} сезонов",
  "title.specialCountOne": "{{count}} спецвыпуск",
  "title.specialCountOther": "{{count}} спецвыпусков",
  "title.movieCountOne": "{{count}} фильм",
  "title.movieCountOther": "{{count}} фильмов",
  "title.movieDetails": "Сведения о фильме",
  "title.contextReleaseSearchSummary":
    "{{releaseCount}} релизов · выполнен поиск по {{indexerCount}} индексаторам",
  "title.contextReleaseSearchProgress":
    "{{releaseCount}} релизов · {{done}}/{{total}} индексаторов",
  "title.contextReleaseSearchSummaryDetailed":
    "{{releaseCount}} релизов · поиск выполнен по {{searched}} из {{total}} индексаторов",
  "title.contextReleaseSearchIndexerFailures":
    "Ошибка в {{count}} индексаторах: {{names}}",
  "title.contextReleaseSearchIndexerSkipped":
    "Пропущено индексаторов: {{count}} ({{names}})",
  "title.noReleasesFound": "Для {{name}} релизы не найдены.",
  "title.searchNeedsDownloadClientTitle":
    "Для поиска требуется клиент загрузки",
  "title.searchNeedsDownloadClientDescription":
    "Настройте хотя бы один клиент загрузки, прежде чем использовать поиск или интерактивный поиск на этой странице.",
  "title.searchNeedsDownloadClientAction": "Открыть раздел «Клиенты загрузки»",
  "title.blockedReleases": "Заблокированные релизы",
  "title.noBlockedReleases":
    "Для этого фильма нет записей о заблокированных релизах.",
  "title.settings": "Настройки материала",
  "title.qualityProfile": "Профиль качества",
  "title.rootFolder": "Корневая папка",
  "title.catalogFilters.allRootFolders": "Все корневые папки",
  "title.catalogFilters.selectedCount": "Выбрано: {{count}}",
  "title.catalogFilters.minimumYear": "Минимальный год выпуска",
  "title.catalogFilters.maximumYear": "Максимальный год выпуска",
  "title.catalogFilters.loadError": "Не удалось обновить параметры фильтра.",
  "title.catalogFilters.retry": "Повторить загрузку параметров фильтра",
  "title.defaultRootFolder": "По умолчанию ({{path}})",
  "title.inheritDefault": "Наследовать значение по умолчанию",
  "title.monitoredSection": "Отслеживаемые материалы: {{facet}}",
  "title.noWantedItem": "Нет активных желаемых материалов.",
  "title.searchReleasesAction": "Искать релизы",
  "title.queueLatest": "Добавить последний релиз в очередь",
  "title.requiredAudioLanguages": "Требуемые языки аудио",
  "title.originalAudioLanguagePerTitle":
    "Язык оригинала (для каждого произведения)",
  "title.requiredAudioLanguagesFacetInfo":
    "Эти языки учитываются при выборе релиза и проверке импортируемых файлов для данной категории.",
  "title.requiredAudioInherited": "Наследуется из настроек категории {{facet}}",
  "title.requiredAudioResetInherit": "Сбросить и наследовать",
  "title.fixMatchHeading": "Исправить соответствие метаданных",
  "title.fixMatchAction": "Исправить соответствие",
  "title.fixMatchDescriptionMovie":
    "Исправить соответствие фильма в TVDB, сохранив локальное название и имя папки без изменений.",
  "title.fixMatchDescriptionSeries":
    "Исправить соответствие в TVDB для этого материала. Локальное название и имя папки останутся без изменений.",
  "title.fixMatchDialogTitle": "Исправление соответствия",
  "title.fixMatchDialogDescription":
    "Исправить соответствие в TVDB для {{name}}. Локальное название и имя папки останутся без изменений. Для сериалов и аниме сезоны и эпизоды будут перестроены автоматически.",
  "title.fixMatchUnnamed": "этого материала",
  "title.fixMatchSearchPlaceholder": "Поиск материалов в TVDB",
  "title.fixMatchCurrentTvdbId": "Текущий идентификатор TVDB",
  "title.fixMatchCurrentTvdbNone": "отсутствует",
  "title.fixMatchSearching": "Поиск метаданных…",
  "title.fixMatchNoResults": "Совпадений по метаданным не найдено.",
  "title.fixMatchSelected": "Выбрано",
  "title.fixMatchChoose": "Выбрать",
  "title.fixMatchApplying": "Исправление соответствия…",
  "title.fixMatchApply": "Применить соответствие",
  "title.fixMatchSearchFailed": "Не удалось выполнить поиск метаданных.",
  "title.fixMatchApplyFailed": "Не удалось исправить соответствие материала.",
  "title.changeFolderHeading": "Изменить папку",
  "title.changeFolderDescription":
    "Исправьте, какой существующей папкой владеет этот материал. Файлы не перемещаются.",
  "title.changeFolderAction": "Изменить папку",
  "title.changeFolderDialogTitle": "Изменить папку",
  "title.changeFolderDialogDescription":
    "Выберите папку, которая относится к «{{name}}». Scryer переназначит владение и повторно просканирует папку; файлы не перемещаются.",
  "title.changeFolderLibrary": "Библиотека",
  "title.changeFolderCurrentFolder": "Текущая папка",
  "title.changeFolderCurrentFolderUnknown": "Отобразится после выбора папки",
  "title.changeFolderCurrentRoot": "Текущий корневой каталог",
  "title.changeFolderCurrentRootUnknown": "Неизвестно",
  "title.changeFolderRootLabel": "Корневой каталог",
  "title.changeFolderRootScopeHelp":
    "Выбрать можно только папки внутри корневых каталогов библиотеки этого материала.",
  "title.changeFolderNoRoots":
    "В библиотеке этого материала нет корневых каталогов для просмотра.",
  "title.changeFolderBrowseLoading": "Загрузка папок…",
  "title.changeFolderBrowseEmpty": "Здесь нет вложенных папок.",
  "title.changeFolderBrowseFailed": "Не удалось получить список папок.",
  "title.changeFolderUp": "На уровень выше",
  "title.changeFolderOpen": "Открыть",
  "title.changeFolderSelectPrompt":
    "Выберите папку, чтобы увидеть предварительный просмотр изменения.",
  "title.changeFolderPreviewLoading": "Проверка выбранной папки…",
  "title.changeFolderPreviewFailed":
    "Не удалось выполнить предварительный просмотр смены папки.",
  "title.changeFolderSelectedFolder": "Выбранная папка",
  "title.changeFolderOwnershipUnowned":
    "Эта папка не принадлежит ни одному материалу.",
  "title.changeFolderOwnershipThisTitle":
    "Этот материал уже владеет данной папкой.",
  "title.changeFolderOwnershipAnother": "Этой папкой владеет «{{name}}».",
  "title.changeFolderNoOpHelp":
    "Ничего не будет отправлено. Выберите другую папку или закройте это окно.",
  "title.changeFolderTrackedMediaCurrent":
    "Отслеживаемых медиафайлов в текущей папке: {{count}}",
  "title.changeFolderTrackedMediaSelected":
    "Отслеживаемых медиафайлов в выбранной папке: {{count}}",
  "title.changeFolderNoFilesMoved":
    "Файлы не будут перемещены. Изменятся только соответствие папки и производные от него связи медиафайлов.",
  "title.changeFolderConflictHeading":
    "Эта папка уже принадлежит другому материалу",
  "title.changeFolderChooseResolution":
    "Выберите «Обменять папки» или «Забрать папку», чтобы продолжить.",
  "title.changeFolderSwapAction": "Обменять папки",
  "title.changeFolderSwapExplanation":
    "«{{owner}}» получает текущую папку этого материала, а этот материал получает {{folder}}. Оба материала будут просканированы повторно.",
  "title.changeFolderTakeOverAction": "Забрать папку",
  "title.changeFolderTakeOverExplanation":
    "Этот материал получает {{folder}}. «{{owner}}» останется без папки, станет несопоставленным и потребует восстановления в разделе «Ожидающие импорта».",
  "title.changeFolderConfirm": "Изменить папку",
  "title.changeFolderApplying": "Изменение папки…",
  "title.changeFolderApplyFailed": "Не удалось изменить соответствие папки.",
  "title.changeFolderOutcomeAssigned": "«{{name}}» теперь владеет {{folder}}.",
  "title.changeFolderOutcomeSwapped":
    "«{{name}}» и «{{other}}» обменялись папками.",
  "title.changeFolderOutcomeAlreadyOwned":
    "«{{name}}» уже владел этой папкой; ничего не изменилось.",
  "title.changeFolderDetachedMedia":
    "Отсоединено связей медиафайлов: {{count}}.",
  "title.changeFolderScanSummary":
    "Сканирование: просмотрено {{scanned}} • сопоставлено {{matched}} • импортировано {{imported}} • без соответствия {{unmatched}}",
  "title.changeFolderDisplacedNote":
    "«{{name}}» потерял папку {{folder}} и теперь не сопоставлен. Восстановите его в разделе «Ожидающие импорта», где он отображается с причиной «Владение папкой изменено пользователем».",
  "pendingImports.title": "Ожидающие импорта",
  "pendingImports.description":
    "Устраните несопоставленные элементы на диске для категории {{facet}}, выбрав правильное соответствие в метаданных.",
  "pendingImports.unresolvedCountOne": "1 неразрешённый элемент",
  "pendingImports.unresolvedCountOther": "{{count}} неразрешённых элементов",
  "pendingImports.loading": "Загрузка ожидающих импорта…",
  "pendingImports.loadFailed": "Не удалось загрузить ожидающие импорты.",
  "pendingImports.empty": "Для этой категории нет ожидающих импортов.",
  "pendingImports.path": "Путь",
  "pendingImports.folderPath": "Папка",
  "pendingImports.searchPlaceholder": "Поиск материалов в метаданных",
  "pendingImports.searching": "Поиск метаданных…",
  "pendingImports.searchFailed": "Не удалось выполнить поиск по метаданным.",
  "pendingImports.noSearchResults": "Совпадений в метаданных не найдено.",
  "pendingImports.searchAction": "Найти и сопоставить",
  "pendingImports.match": "Сопоставить",
  "pendingImports.ignoredSection": "Игнорируемые импорты",
  "pendingImports.ignoredEmpty":
    "Для этой категории нет игнорируемых импортов.",
  "pendingImports.ignore": "Игнорировать",
  "pendingImports.ignoreConfirmTitle": "Игнорировать импорт?",
  "pendingImports.ignoreSuccess": "Импорт для {{name}} был проигнорирован.",
  "pendingImports.ignoreFailed": "Не удалось проигнорировать ожидающий импорт.",
  "pendingImports.resolving": "Обработка…",
  "pendingImports.resolveSuccess":
    "Ожидающий импорт для {{name}} успешно обработан.",
  "pendingImports.resolveFailed": "Не удалось обработать ожидающий импорт.",
  "pendingImports.prev": "Предыдущая",
  "pendingImports.next": "Следующая",
  "pendingImports.pageRange": "{{start}}–{{end}} / {{total}}",
  "pendingImports.library": "Библиотека:",
  "pendingImports.knownTitle": "Известное название:",
  "pendingImports.reason.title_already_owns_another_folder":
    "Материал уже связан с другой папкой",
  "pendingImports.reason.folder_ownership_changed_by_user":
    "Владение папкой изменено пользователем",
  "pendingImports.folderOwnershipChangedHelp":
    "Эта папка была передана другому материалу, поэтому у текущего материала папки больше нет. Откройте материал и укажите правильную папку через «Изменить папку» либо скройте этот элемент с помощью «Игнорировать».",
  "pendingImports.ownershipConflictHelp":
    "Этот тайтл связан с папкой {{folder}}. Переместите файлы из этой папки туда или удалите дублирующую папку, затем повторите сканирование.",
  "pendingImports.bindEpisodes": "Связать эпизоды",
  "pendingImports.bindPreviewLoadFailed":
    "Не удалось загрузить предварительный просмотр связывания",
  "pendingImports.bindFailed": "Не удалось связать ожидающий импорт.",
  "pendingImports.selectAtLeastOneEpisode": "Выберите хотя бы один эпизод.",
  "pendingImports.loadingEpisodeBindings": "Загрузка связей эпизодов...",
  "pendingImports.parsedHints": "Распознанные данные:",
  "pendingImports.parsedHintSeason": " сезон {{season}}",
  "pendingImports.parsedHintEpisodes": " • эпизоды {{episodes}}",
  "pendingImports.parsedHintAbsolute": " • абсолютная нумерация {{absolute}}",
  "pendingImports.useSuggested": "Использовать предложенное",
  "pendingImports.specials": "Спецвыпуски",
  "pendingImports.seasonNumber": "Сезон {{number}}",
  "pendingImports.selectSeason": "Выбрать сезон",
  "pendingImports.bindSelectedEpisodes": "Связать выбранные эпизоды",
  "pendingImports.episodeNumberLabel": "Эпизод {{number}}",
  "tvdb.searchResults": "Результаты поиска TVDB",
  "tvdb.searchPrompt": "Введите название выше, чтобы выполнить поиск в TVDB.",
  "tvdb.searching": "Поиск метаданных TVDB для «{{query}}»",
  "tvdb.foundMatches": "Найдено совпадений в TVDB: {{count}}.",
  "tvdb.noMatches": "Совпадений в TVDB не найдено.",
  "tvdb.selectPrompt": "Выберите результат из TVDB, чтобы продолжить.",
  "tvdb.searchByTvdb": "Поиск в TVDB",
  "tvdb.searchButton": "Поиск",
  "tvdb.select": "Выбрать",
  "tvdb.popularity": "Популярность",
  "nzb.searchResults": "Результаты поиска в NZBGeek",
  "nzb.searchResultsFor": "Результаты NZB для {{name}}",
  "nzb.selectTvdbFirst":
    "Сначала выберите результат из TVDB, чтобы выполнить поиск в NZBGeek.",
  "nzb.noResultsYet": "Совпадений среди релизов пока нет.",
  "nzb.searchFailed": "Ошибка поиска релизов.",
  "nzb.blockedByProfile": "Заблокировано профилем: {{reason}}",
  "nzb.queue": "Добавить в очередь",
  "nzb.queueAdditionalFile": "Добавить в очередь как дополнительный файл",
  "nzb.blocked": "Заблокировано",
  "nzb.noConsideredResults": "Ни один релиз не соответствует профилю качества.",
  "nzb.blockedResults": "{{count}} заблокированных релизов",
  "nzb.showScoringLog": "Показать журнал оценки",
  "nzb.hideScoringLog": "Скрыть журнал оценки",
  "nzb.score": "Оценка",
  "nzb.size": "Размер",
  "nzb.scoringLog": "Журнал оценки",
  "nzb.total": "Итого",
  "media.posterAlt": "Постер: {{name}}",
  "activity.title": "Активность",
  "activity.loading": "Загрузка активности…",
  "activity.noActivity": "Данные об активности не загружены.",
  "activity.actor": "пользователь {{actor}}",
  "activity.actorUnknown": "пользователь неизвестен",
  "activity.timeUnknown": "время неизвестно",
  "activity.loadedTemplate": "Загружено {{count}} записей активности.",
  "activity.rowActor": "{{actor}}",
  "activity.noSystemActor": "пользователь неизвестен",
  "activity.noSystemTime": "время неизвестно",
  "activity.tabToggleLabel": "Вкладка активности загрузок",
  "activity.filterBarLabel": "Фильтр активности загрузок",
  "activity.import": "Импорт",
  "activity.activity": "Активность",
  "activity.allActivity": "Вся активность",
  "activity.history": "История",
  "activity.manualImportRequired": "Требуется ручной импорт",
  "activity.manualImportRequiredDescription":
    "Этим загрузкам требуется быстрое сопоставление с материалом или ручной импорт, прежде чем они будут завершены.",
  "activity.importFilter.all": "Все",
  "activity.importFilter.importing": "Импортируется",
  "activity.importFilter.pending": "Ожидание",
  "activity.importFilter.blocked": "Заблокировано",
  "activity.importFilter.failed": "Ошибка",
  "activity.activityFilter.all": "Все",
  "activity.activityFilter.downloading": "Загружается",
  "activity.activityFilter.queued": "В очереди",
  "activity.activityFilter.paused": "Приостановлено",
  "activity.activityFilter.postProcessing": "Постобработка",
  "activity.activityFilter.seeding": "Раздаётся",
  "activity.historyFilter.all": "Все",
  "activity.historyFilter.success": "Успешно",
  "activity.historyFilter.failed": "Ошибка",
  "activity.scryerSubmitted": "Отправлено Scryer",
  "activity.selectAllImportItems": "Выбрать все элементы импорта",
  "activity.selectImportItem": "Выбрать элемент импорта",
  "activity.selectedImportCount": "Выбрано: {{count}}",
  "activity.importEmpty": "Нет элементов для импорта.",
  "activity.activityEmpty": "Нет активных загрузок.",
  "activity.historyEmpty": "История пуста.",
  "activity.queueStale":
    "Данные очереди устарели. Отображаются результаты последнего успешного обновления.",
  "queue.title": "Название",
  "queue.client": "Клиент",
  "queue.status": "Статус",
  "queue.source": "Источник",
  "queue.progress": "Прогресс",
  "queue.size": "Размер",
  "queue.state.queued": "В очереди",
  "queue.state.downloading": "Загружается",
  "queue.state.verifying": "Проверка",
  "queue.state.repairing": "Восстановление",
  "queue.state.extracting": "Распаковка",
  "queue.state.postProcessing": "Постобработка",
  "queue.state.paused": "Приостановлено",
  "queue.state.completed": "Завершено",
  "queue.state.importing": "Импортируется",
  "queue.state.importPending": "Ожидает импорта",
  "queue.state.importBlocked": "Импорт заблокирован",
  "queue.state.failed": "Ошибка",
  "queue.state.ignored": "Игнорируется",
  "queue.state.unknown": "Неизвестно",
  "queue.empty": "Нет активных или ожидающих загрузок.",
  "queue.error": "Не удалось загрузить очередь загрузок.",
  "queue.refresh": "Обновить",
  "queue.lastRefresh": "Последнее обновление: {{time}}",
  "queue.summary":
    "{{downloading}} загружается, {{waiting}} ожидают импорта, {{attention}} требуют внимания",
  "queue.manualImport": "Ручной импорт",
  "queue.manualImporting": "Постановка в очередь…",
  "queue.transfer.copying": "Копирование",
  "queue.transfer.extracting": "Извлечение",
  "queue.transfer.finalizing": "Завершение",
  "queue.manualImportQueued": "Ручной импорт добавлен в очередь.",
  "queue.manualImportFailed": "Ошибка ручного импорта.",
  "queue.manualImportMultipleEpisodes": "Несколько эпизодов в этом файле",
  "queue.manualImportMultipleEpisodesLabel":
    "Назначить этому файлу несколько эпизодов",
  "queue.manualImportApplySuggestions": "Применить предложения ко всем",
  "queue.removeFailed": "Удалить ошибочные",
  "queue.manualImportTooltip":
    "Ручной импорт доступен для элементов очереди, требующих внимания.",
  "queue.releaseTitle": "Релиз",
  "queue.blockReason": "Причина блокировки",
  "queue.blockReasonFallbackUnassigned":
    "Автоматический импорт не смог определить материал в библиотеке. Назначьте материал, чтобы продолжить.",
  "queue.blockReasonFallbackEpisodic":
    "Автоматический импорт не смог определить уникальное соответствие сезона и эпизода. Откройте ручной импорт и укажите правильные сезон и эпизод.",
  "queue.blockReasonFallbackReview":
    "Автоматический импорт требует проверки. Откройте ручной импорт и подтвердите сопоставление файлов, чтобы продолжить.",
  "queue.errorCode": "Код ошибки",
  "queue.showDetails": "Показать подробности",
  "queue.hideDetails": "Скрыть подробности",
  "queue.assignTitle": "Назначить материал",
  "queue.reassignTitle": "Изменить материал",
  "queue.assignTitleTitle": "Назначение материала для загрузки",
  "queue.assignTitleDescription":
    "Выберите материал из библиотеки, которому принадлежит эта заблокированная загрузка. После назначения Scryer повторно проверит импорт.",
  "queue.assignTitlePlaceholder": "Поиск материалов...",
  "queue.assignTitleResults": "Подходящие материалы",
  "queue.assignTitleEmpty": "Подходящие материалы не найдены.",
  "queue.assignTitleQueued":
    "Материал назначен. Импорт будет повторно проверен.",
  "queue.assignTitleFailed": "Не удалось назначить материал.",
  "queue.assignTitleBeforeImport":
    "Назначьте материал перед запуском ручного импорта.",
  "queue.ignore": "Игнорировать",
  "queue.ignoreSuccess": "Загрузка проигнорирована.",
  "queue.ignoreFailed": "Не удалось проигнорировать загрузку.",
  "queue.releaseHeldSources": "Освободить удерживаемые источники",
  "queue.releaseHeldSourcesConfirmTitle": "Освободить удерживаемые источники?",
  "queue.releaseHeldSourcesConfirmDescription":
    "{{download}} в {{client}} импортирован, но источники всё ещё удерживаются. Освобождение снимает удержание со всех импортов этой загрузки. Временные папки распаковки удаляются, только если все видео в них импортированы и они больше нигде не используются.",
  "queue.releaseHeldSourcesClientRemoves":
    "{{client}} настроен удалять завершённые загрузки. Как только Scryer подтвердит импорт, эта загрузка и её файлы будут удалены из клиента загрузок.",
  "queue.releaseHeldSourcesClientKeeps":
    "{{client}} настроен сохранять завершённые загрузки, поэтому эта загрузка останется в клиенте загрузок.",
  "queue.releaseHeldSourcesSuccess": "Удерживаемые источники освобождены.",
  "queue.releaseHeldSourcesFailed":
    "Не удалось освободить удерживаемые источники.",
  "queue.releaseHeldSourcesReasonSubtitlesPending":
    "Источники сохраняются из-за субтитров, которые ещё требуют внимания. Освобождение прекращает ожидание и снимает предупреждение.",
  "queue.releaseHeldSourcesReasonSourceCleanupIncomplete":
    "Источники сохраняются, потому что удаление файлов, заменённых этим импортом, не завершилось.",
  "queue.releaseHeldSourcesReasonArchiveExtractionFailed":
    "Не удалось распаковать архивы этой загрузки, и нераспакованные архивы так и не были импортированы. Освобождение отмечает загрузку импортированной, позволяя клиенту загрузок удалить её, только если доказано, что импортирован каждый ожидаемый файл; всё, что не было импортировано, удаляется вместе с ней.",
  "queue.releaseHeldSourcesReasonUnknown":
    "У Scryer нет записи о том, почему эти источники удерживаются. Освобождение отмечает загрузку импортированной, позволяя клиенту загрузок удалить её, только если доказано, что импортирован каждый ожидаемый файл; всё, что не было импортировано, удаляется вместе с ней.",
  "queue.releaseHeldSourcesTitles": "Освобождение затрагивает: {{titles}}",
  "queue.releaseHeldSourcesClientRemovesAfterSeeding":
    "{{client}} удаляет завершённые загрузки после выполнения требований раздачи. Как только Scryer подтвердит импорт, эта загрузка и её файлы будут удалены по окончании раздачи.",
  "queue.releaseHeldSourcesClientPolicyUnknown":
    "Scryer не удалось определить, что {{client}} сделает с этой загрузкой после освобождения.",
  "queue.releaseHeldSourcesAwaitingImport":
    "Удерживаемые источники освобождены. Загрузка импортирована не полностью, поэтому она возвращена к импорту и ничего не удалено.",
  "queue.releaseHeldSourcesNotSettled":
    "Удерживаемые источники освобождены, но загрузку не удалось обновить. Освободите ещё раз, чтобы завершить.",
  "queue.releaseHeldSourcesUnproven":
    "Удерживаемые источники освобождены, но ничто не доказывает, что импортирован каждый файл загрузки, поэтому она не отмечена импортированной и ничего не очищено. Импортируйте остальное или удалите её сами.",
  "queue.releaseHeldSourcesNotSettledNothingCleaned":
    "Удерживаемые источники освобождены, но загрузка не была завершена, и ничего не было очищено.",
  "queue.releaseHeldSourcesWorkspacesPreserved":
    "Некоторые временные папки распаковки сохранены: {{reasons}}.",
  "queue.releaseHeldSourcesPreservedNotOwned":
    "не удалось подтвердить владельца",
  "queue.releaseHeldSourcesPreservedUnsafe":
    "неожиданное или нечитаемое содержимое",
  "queue.releaseHeldSourcesPreservedHoldsUnimportedVideo":
    "содержит видео, которое не было импортировано",
  "queue.releaseHeldSourcesPreservedInUse": "ещё нужна другому импорту",
  "queue.releaseHeldSourcesPreservedUnverified":
    "не удалось подтвердить импортированные файлы",
  "queue.releaseHeldSourcesPreservedRemovalFailed": "не удалось удалить",
  "queue.releaseHeldSourcesPreservedDownloadNotImported":
    "импорт загрузки не подтверждён",
  "queue.releaseHeldSourcesPreservedLookupIncomplete":
    "не все папки удалось проверить",
  "queue.markFailedSearchAgain": "Заблокировать, удалить данные и искать снова",
  "queue.markFailedOnly": "Заблокировать и удалить данные",
  "queue.markFailedSearchSuccess":
    "Запрошены блокировка и удаление данных; повторный поиск включён.",
  "queue.markFailedOnlySuccess":
    "Запрошены блокировка и удаление данных без повторного поиска.",
  "queue.markFailedFailed": "Не удалось пометить загрузку как ошибочную.",
  "queue.bulkIgnoreSuccess": "Проигнорировано загрузок: {{count}}.",
  "queue.bulkIgnorePartial":
    "Проигнорировано загрузок: {{count}}, ошибок: {{failed}}.",
  "queue.bulkIgnoreFailed": "Не удалось проигнорировать выбранные загрузки.",
  "queue.pause": "Приостановить",
  "queue.pausing": "Приостановка…",
  "queue.pauseSuccess": "Загрузка приостановлена.",
  "queue.pauseFailed": "Не удалось приостановить загрузку.",
  "queue.resume": "Продолжить",
  "queue.resuming": "Возобновление…",
  "queue.resumeSuccess": "Загрузка возобновлена.",
  "queue.resumeFailed": "Не удалось возобновить загрузку.",
  "queue.deleting": "Удаление…",
  "queue.deleteSuccess": "Загрузка удалена.",
  "queue.deleteQueued": "Удаление из клиента загрузки поставлено в очередь.",
  "queue.deleteFailed": "Не удалось удалить загрузку из клиента загрузки.",
  "queue.removeFromDownloader": "Удалить из клиента загрузки",
  "queue.deleteConfirmTitle": "Удаление из клиента загрузки",
  "queue.deleteConfirmDescription":
    "Удалить эту загрузку из клиента? После подтверждения удаления она будет проигнорирована.",
  "queue.bulkDeleteQueued":
    "Удаление из клиента поставлено в очередь для {{count}} загрузок.",
  "queue.bulkDeletePartial":
    "Удаление поставлено в очередь для {{count}} загрузок; ошибок: {{failed}}.",
  "queue.bulkDeleteFailed": "Не удалось удалить выбранные загрузки.",
  "queue.bulkDeleteConfirmTitle": "Удалить выбранные загрузки из клиента",
  "queue.bulkDeleteConfirmDescription":
    "Удалить {{count}} выбранных загрузок из клиента? После успешного удаления они будут проигнорированы.",
  "queue.manualUnavailableForResult":
    "Ручная обработка недоступна для этого результата.",
  "importHistory.title": "История импорта",
  "importHistory.empty": "Записи об импорте не найдены.",
  "importHistory.status": "Статус",
  "importHistory.sourceRef": "Источник",
  "importHistory.decision": "Решение",
  "importHistory.error": "Ошибка",
  "importHistory.createdAt": "Дата",
  "importHistory.retry": "Повторить импорт",
  "importHistory.retryHelp":
    "Повторно проверяет уже загруженные файлы с текущим назначением и профилем качества. Не запускает повторную загрузку.",
  "importHistory.retrying": "Повторная попытка...",
  "importHistory.retryWithPassword": "Повторить импорт с паролем",
  "importHistory.passwordRequired": "Требуется пароль",
  "importHistory.passwordPlaceholder": "Пароль архива",
  "importHistory.retrySuccess": "Повторный импорт запущен",
  "system.title": "Состояние системы",
  "system.jobsTitle": "Задания",
  "system.auditTitle": "Аудит",
  "system.auditLoaded": "Журнал аудита загружен.",
  "system.auditEmpty": "События аудита не найдены.",
  "system.auditTarget": "Объект",
  "system.auditStream": "Поток",
  "system.notLoaded": "Состояние системы не загружено.",
  "system.refreshing": "Обновление",
  "system.loaded": "Состояние системы загружено.",
  "system.notReady": "Служба не готова.",
  "system.serviceReady": "Служба готова",
  "system.dbPath": "Путь к базе данных",
  "system.totalTitles": "Всего материалов",
  "system.monitoredTitles": "Отслеживаемые материалы",
  "system.users": "Пользователи",
  "system.facetBreakdown": "Категории",
  "system.recentEvents": "Примеры последних событий",
  "system.loadedTemplate": "Обзор системы загружен.",
  "system.monitoredTitlesLabel": "Отслеживаемые материалы",
  "system.dbPathLabel": "Путь к базе данных",
  "system.totalTitlesLabel": "Всего материалов",
  "system.usersLabel": "Пользователи",
  "system.facetLabel": "Категории",
  "system.recentEventSample": "Примеры последних событий",
  "system.sourcesTitle": "Источники данных",
  "system.sourcesSupport":
    "Если эти сервисы помогают поддерживать работу вашей библиотеки, рассмотрите возможность поддержать их пожертвованием.",
  "system.sourceTvdbName": "TheTVDB",
  "system.sourceTvdbDescription":
    "Основной источник метаданных о телесериалах и эпизодах.",
  "system.sourceTmdbName": "The Movie Database (TMDB)",
  "system.sourceTmdbDescription":
    "Дополнительный источник метаданных о фильмах и сериалах.",
  "system.sourceJikanName": "Jikan API",
  "system.sourceJikanDescription":
    "API для работы с аниме, используемый для синхронизации метаданных MAL, связанных с филлерами и краткими пересказами.",
  "system.sourceMalName": "MyAnimeList",
  "system.sourceMalDescription":
    "Основной набор данных об аниме, предоставляемый через Jikan.",
  "system.sourceAniBridgeName": "AniBridge",
  "system.sourceAniBridgeDescription":
    "Набор данных для сопоставления аниме, используемый для синхронизации идентификаторов из разных источников и соответствия эпизодов.",
  "settings.description":
    "Заполнители разделов для индексаторов, клиентов и уведомлений.",
  "settings.description2":
    "Этот интерфейс готов к добавлению новых форм настроек и может быть расширен без изменения навигации.",
  "settings.knownUsers": "Известные пользователи",
  "settings.user": "Пользователь",
  "settings.createUser": "Создать пользователя",
  "settings.username": "Имя пользователя",
  "settings.password": "Пароль",
  "settings.permissions": "Разрешения",
  "settings.noUsers": "Пользователи не загружены.",
  "settings.savePermissions": "Сохранить разрешения",
  "settings.updatePassword": "Обновить",
  "settings.currentPermissions": "Текущие разрешения",
  "settings.apiKey": "API-ключ",
  "settings.indexerProviderSection": "Провайдеры индексаторов",
  "settings.existingIndexers": "Существующие индексаторы",
  "settings.indexerProvider": "Провайдер",
  "settings.indexerLimits": "Ограничения",
  "settings.indexerInteractiveSearch": "Интерактивный",
  "settings.indexerAutoSearch": "Автоматический",
  "settings.indexerStatus": "Статус",
  "settings.indexerDisabledUntil": "Отключено до {{time}}",
  "settings.indexerCoolingDownUntil": "Пауза · возобновится {{time}}",
  "settings.indexerCoolingDownHelp":
    "Индексер попросил Scryer снизить темп. Поиск возобновится автоматически.",
  "settings.indexerLastError": "Ошибка: {{time}}",
  "settings.indexerLastSearched": "Последний поиск: {{time}}",
  "settings.indexerNoActivity": "Нет активности",
  "settings.indexerSyncNow": "Синхронизировать",
  "settings.indexerAdd": "Добавить индексатор",
  "settings.indexerUpdate": "Обновить индексатор",
  "settings.indexerCreate": "Подключить индексатор",
  "settings.indexerCreateNew": "Подключить индексатор",
  "settings.indexerManagedParentHint":
    "Родительские конфигурации Prowlarr используются только для управления. Режимы поиска наследуются от синхронизированных дочерних индексаторов.",
  "settings.indexerConfirmDiscardTitle":
    "Отменить текущие изменения индексатора?",
  "settings.indexerConfirmDiscardDescription":
    "Все внесённые изменения будут потеряны.",
  "settings.indexerFilterPlaceholder": "Фильтр по типу провайдера",
  "settings.noIndexersFound": "Индексаторы не найдены.",
  "settings.indexerConfig": "Настройки провайдера",
  "settings.indexerDynamicFieldsLoading": "Загрузка полей для этого выбора…",
  "settings.indexerDynamicFieldsError":
    "Не удалось загрузить поля для этого выбора: {{error}}",
  "settings.indexerLoginChallengeLoading": "Загрузка изображения для входа…",
  "settings.indexerLoginChallengeImageAlt": "Изображение для входа",
  "settings.indexerLoginChallengeError":
    "Не удалось загрузить изображение для входа: {{error}}",
  "settings.indexerLoginChallengeRefresh": "Новое изображение",
  "settings.indexerLoginChallengeRefusal.malformed":
    "Не удалось прочитать изображение для входа от плагина.",
  "settings.indexerLoginChallengeRefusal.kindUnsupported":
    "Этот индексатор требует проверку входа, которую Scryer не может показать.",
  "settings.indexerLoginChallengeRefusal.imageTypeUnsupported":
    "Изображение для входа в формате, который Scryer не показывает.",
  "settings.indexerLoginChallengeRefusal.imageTooLarge":
    "Изображение для входа слишком большое для показа.",
  "settings.managedIndexerBadge": "Управляемый",
  "settings.managedByIndexer": "Управляется: {{name}}",
  "settings.managedByParent": "Управляется родительским элементом",
  "settings.managesIndexerCount": "Управляет {{count}} индексаторами",
  "settings.managedIndexerReadOnly":
    "Управляемые индексаторы контролируются родительской синхронизацией и не могут быть изменены напрямую.",
  "settings.managedIndexerReadOnlyShort": "Только чтение",
  "settings.indexerDownloadClient": "Клиент загрузки",
  "settings.indexerDownloadClientLabel": "Клиент загрузки для {{name}}",
  "settings.indexerDownloadClientAutomatic": "Автоматически",
  "settings.indexerDownloadClientLoading": "Загрузка клиентов...",
  "settings.indexerDownloadClientLoadRetry": "Повторить загрузку клиентов",
  "settings.indexerDownloadClientCatalogStale":
    "Список клиентов загрузки может быть устаревшим.",
  "settings.indexerDownloadClientSelectionInvalid":
    "Выберите режим «Автоматически» или совместимый клиент загрузки.",
  "settings.indexerDownloadClientNotApplicable": "Неприменимо",
  "settings.indexerDownloadClientDisabled": "Отключено",
  "settings.indexerDownloadClientDisabledWarning":
    "{{name}} отключён. Захват релизов будет недоступен, пока он не будет включён.",
  "settings.indexerDownloadClientChooseAutomatic":
    "Выбрать автоматический режим",
  "settings.indexerDownloadClientInvalidOption":
    "Недопустимый вариант: {{name}}",
  "settings.indexerDownloadClientInvalidMissing":
    "Связанный клиент загрузки отсутствует. Выберите действительный клиент или автоматический режим.",
  "settings.indexerDownloadClientInvalidIncompatible":
    "Связанный клиент загрузки несовместим с этим индексатором.",
  "settings.indexerDownloadClientInvalidUnavailable":
    "Связанный клиент загрузки недоступен. Выберите действительный клиент или автоматический режим.",
  "settings.downloadClientsSection": "Клиенты загрузки",
  "settings.downloadClientDeleteConfirmDescription":
    "Удалить {{name}}? Все связанные с ним индексаторы будут переведены в автоматический режим.",
  "settings.downloadClientFilterPlaceholder":
    "Фильтр по названию или типу клиента",
  "settings.newPassword": "Новый пароль",
  "settings.passwordManagedExternally": "Управляется внешней системой",
  "settings.mfa": "MFA",
  "settings.passkey": "Ключ доступа",
  "settings.setUp": "Настроить",
  "settings.notSetUp": "Не настроено",
  "settings.resetMfa": "Сбросить MFA",
  "settings.resetMfaConfirm":
    "Сбросить MFA для пользователя {{name}}? При следующем входе ему потребуется повторно настроить MFA.",
  "settings.loginEnabled": "Вход разрешён",
  "settings.loginDisabled": "Вход запрещён",
  "settings.enableLogin": "Разрешить вход",
  "settings.disableLogin": "Запретить вход",
  "settings.disableLoginConfirm":
    "Отключить вход для пользователя {{name}}? Его учётные данные и методы аутентификации будут сохранены, но все активные сеансы будут завершены.",
  "settings.save": "Сохранить",
  "settings.noPermissions": "Нет разрешений",
  "settings.filter": "Фильтр",
  "settings.rules": "Пользовательские правила",
  "settings.rulesSection": "Пользовательские правила оценки",
  "settings.existingRules": "Существующие правила",
  "settings.noRulesFound": "Пользовательские правила не настроены.",
  "settings.trashPacksTitle": "Языковые пакеты TRaSH Guides",
  "settings.trashPacksHelp":
    "Пакеты оценки, основанные на рекомендациях TRaSH Guides. Выберите пакет, чтобы применить заранее настроенную языковую политику.",
  "settings.trashPackFilterLabel": "Фильтр тегов",
  "settings.trashPackFilterPlaceholder": "locale:french, locale:vf",
  "settings.trashPackFilterHelp":
    "Оставьте поле пустым, чтобы применять этот пакет ко всем материалам.",
  "settings.trashPackFilterSaved": "Фильтр тегов для {{name}} обновлён.",
  "settings.trashPackFrenchConflict":
    "Пакет «{{name}}» уже включён. Отключите его перед включением другого французского пакета.",
  "settings.ruleCreate": "Создать правило",
  "settings.ruleCreateNew": "Создать новое правило",
  "settings.ruleCopyAsCustom": "Копировать как пользовательское правило",
  "settings.ruleUpdate": "Обновить правило",
  "settings.ruleApplyTemplate": "Применить шаблон",
  "settings.ruleConfirmDiscardTitle": "Отменить текущие изменения правила?",
  "settings.ruleConfirmDiscardDescription":
    "Все внесённые изменения будут потеряны.",
  "settings.ruleDescription": "Описание",
  "settings.ruleRegoSource": "Исходный код Rego",
  "settings.rulePriority": "Приоритет",
  "settings.ruleAppliedFacets": "Применяемые категории",
  "settings.ruleAppliedFacetsHelp":
    "Оставьте поле пустым, чтобы применять правило ко всем типам медиаконтента.",
  "settings.ruleValidate": "Проверить",
  "settings.ruleValidating": "Проверка…",
  "settings.ruleValid": "Правило корректно.",
  "settings.ruleValidationRequired":
    "Необходимо указать название и исходный код Rego.",
  "settings.ruleLibrary": "Библиотека правил",
  "settings.ruleLibraryDescription":
    "Готовые правила, которые можно добавить одним нажатием. Выберите шаблон, чтобы автоматически заполнить редактор.",
  "settings.ruleLibraryAll": "Все",
  "settings.ruleLibraryCommunityEmpty":
    "Пакеты правил сообщества отсутствуют. Обновите реестр плагинов, чтобы проверить наличие новых пакетов.",
  "settings.rulesFacetSection":
    "Пользовательские правила для категории {{facet}}",
  "settings.ruleGlobal": "Глобальные",
  "settings.ruleNoFacetRules":
    "Пользовательские правила отсутствуют. Создайте их в разделе «Настройки → Пользовательские правила».",
  "settings.refTitle": "Справочник входного контекста",
  "settings.refSubtitle":
    "Нажмите, чтобы развернуть полный справочник всех полей, доступных в правилах Rego.",
  "settings.refIntro":
    "Каждое правило получает объект input, содержащий пять документов верхнего уровня. Код Rego обращается к ним через input.release, input.profile, input.context, input.builtin_score и input.file. Объект input.file имеет значение null во время оценки перед загрузкой и заполняется только после загрузки во время проверки импорта.",
  "settings.refColField": "Поле",
  "settings.refColType": "Тип",
  "settings.refColDescription": "Описание",
  "settings.refColFunction": "Функция",
  "settings.refColReturns": "Возвращаемое значение",
  "settings.refSectionRelease": "— Анализ метаданных релиза",
  "settings.refReleaseRawTitle":
    "Исходное необработанное название релиза, полученное от индексатора",
  "settings.refReleaseQuality":
    "Определённый уровень качества (например, 2160P, 1080P, 720P)",
  "settings.refReleaseSource":
    "Источник медиаконтента (например, WEB-DL, BLURAY, HDTV)",
  "settings.refReleaseVideoCodec": "Видеокодек (например, H.265, H.264, AV1)",
  "settings.refReleaseAudio":
    "Основной аудиокодек (например, DDP, AAC, TrueHD)",
  "settings.refReleaseAudioCodecs":
    "Все обнаруженные аудиокодеки в виде массива",
  "settings.refReleaseGuideFacts":
    "Факты и метки, обнаруженные в метаданных релиза",
  "settings.refReleaseAudioChannels":
    "Конфигурация аудиоканалов (например, 5.1, 7.1, 2.0)",
  "settings.refReleaseLangsAudio":
    "Коды языков аудио ISO 639-3 (например, eng, jpn). Включают языки из листинга индексатора, зафиксированные при захвате релиза; файлы, найденные сканированием, принятые или импортированные до того, как начали сохраняться снимки листинга, дают только то, что видно по имени и самому файлу",
  "settings.refReleaseIndexerLanguages":
    "Коды языков аудио ISO 639-3, которые сообщил сам листинг индексатора, зафиксированные при захвате релиза. Никогда не выводятся из имени или тайтла; пусто, если индексатор ничего не сообщил",
  "settings.refReleaseLangsSub": "Коды языков субтитров ISO 639-3",
  "settings.refReleaseIsDualAudio":
    "Истинно, если обнаружено несколько звуковых дорожек",
  "settings.refReleaseIsAtmos": "Истинно, если присутствует звук Dolby Atmos",
  "settings.refReleaseIsDV": "Истинно, если обнаружены метаданные Dolby Vision",
  "settings.refReleaseDetectedHdr":
    "Истинно, если обнаружен HDR (без Dolby Vision)",
  "settings.refReleaseIsRemux":
    "Истинно для полного Remux (без перекодирования)",
  "settings.refReleaseIsBdDisk": "Истинно для источника в виде диска Blu-ray",
  "settings.refReleaseIsProper": "Истинно, если релиз помечен как Proper",
  "settings.refReleaseIsRepack": "Истинно, если релиз помечен как Repack",
  "settings.refReleaseIsAiEnhanced":
    "Истинно, если анализатор обнаружил признак улучшения с помощью ИИ",
  "settings.refReleaseIsHardcodedSubs":
    "Истинно, если в названии релиза обнаружены встроенные субтитры",
  "settings.refReleaseIsPasswordProtected":
    "Истинно, если листинг индексатора пометил релиз как защищённый паролем. Факт листинга индексатора, зафиксированный при захвате релиза; null, если индексатор этого не сообщил, и null для файлов, которые были найдены сканированием, приняты или импортированы до того, как начали сохраняться снимки листинга",
  "settings.refReleaseIsHdr10Plus":
    "Истинно, если в метаданных релиза обнаружен HDR10+",
  "settings.refReleaseIsHlg": "Истинно, если в метаданных релиза обнаружен HLG",
  "settings.refReleaseIs10Bit":
    "Истинно, если в метаданных релиза обнаружено 10-битное видео",
  "settings.refReleaseIsUncensored":
    "Истинно, если релиз помечен как версия без цензуры",
  "settings.refReleaseIsDubsOnly":
    "Истинно, если релиз помечен как содержащий только дубляж",
  "settings.refReleaseHasReleaseGroup":
    "Истинно, если была определена непустая группа релиза",
  "settings.refReleaseIsObfuscated":
    "Истинно, если название релиза выглядит намеренно скрытым или зашифрованным",
  "settings.refReleaseIsRetagged":
    "Истинно, если название релиза выглядит как повторно опубликованное или изменённое",
  "settings.refReleaseStreamingService":
    "Обнаруженная метка потокового сервиса, если она присутствует",
  "settings.refReleaseEdition":
    "Метка издания, например Extended, IMAX или Criterion",
  "settings.refReleaseAnimeVersion":
    "Номер версии аниме, например v2 или v3, если он присутствует",
  "settings.refReleaseEpisodeReleaseType":
    "Тип упаковки эпизодов, например single_episode, multi_episode, season_pack или unknown",
  "settings.refReleaseIsSeasonPack":
    "Истинно, если релиз содержит полный или частичный пакет сезона",
  "settings.refReleaseIsMultiEpisode":
    "Истинно, если релиз содержит несколько эпизодов",
  "settings.refReleaseGroup": "Группа релиза или имя загрузившего пользователя",
  "settings.refReleaseYear": "Год выпуска",
  "settings.refReleaseParseConf":
    "Оценка достоверности анализа (от 0.0 до 1.0)",
  "settings.refReleaseSizeBytes":
    "Размер файла в байтах (для преобразования используйте scryer.size_gib())",
  "settings.refReleaseAgeDays":
    "Полные дни от времени публикации у индексатора до захвата релиза. Факт листинга индексатора, зафиксированный при захвате: возраст на момент захвата, который потом не меняется. Для ещё не захваченного релиза возраст считается на текущий момент. Null, если время публикации неизвестно, и null для файлов, которые были найдены сканированием, приняты или импортированы до того, как начали сохраняться снимки листинга",
  "settings.refReleaseThumbsUp":
    "Положительные голоса индексатора. Факт листинга индексатора, зафиксированный при захвате релиза; null, если индексатор не сообщил голоса, и null для файлов, которые были найдены сканированием, приняты или импортированы до того, как начали сохраняться снимки листинга",
  "settings.refReleaseThumbsDown":
    "Отрицательные голоса индексатора. Факт листинга индексатора, зафиксированный при захвате релиза; null, если индексатор не сообщил голоса, и null для файлов, которые были найдены сканированием, приняты или импортированы до того, как начали сохраняться снимки листинга",
  "settings.refReleaseExtra":
    "Атрибуты листинга, специфичные для индексатора (простые значения и списки значений), доступны через input.release.extra.<key>. Факты листинга индексатора, зафиксированные при захвате релиза; пустой объект для файлов, найденных сканированием, принятых или импортированных до того, как начали сохраняться снимки листинга, поэтому любое обращение к input.release.extra.<key> даёт undefined (а не null), и сравнение вида == null не срабатывает",
  "settings.refSectionProfile": "— Конфигурация активного профиля качества",
  "settings.refProfileId": "Уникальный идентификатор профиля",
  "settings.refProfileName": "Отображаемое имя профиля",
  "settings.refProfileQualityTiers":
    "Упорядоченный список допустимых уровней качества",
  "settings.refProfileArchivalQuality":
    "Наивысший уровень качества, считающийся архивным",
  "settings.refProfileAllowUnknown":
    "Определяет, разрешены ли нераспознанные уровни качества",
  "settings.refProfileSourceAllow":
    "Разрешённые источники (пустое значение означает, что разрешены все)",
  "settings.refProfileSourceBlock": "Заблокированные источники",
  "settings.refProfileVCodecAllow":
    "Разрешённые видеокодеки (пустое значение означает, что разрешены все)",
  "settings.refProfileVCodecBlock": "Заблокированные видеокодеки",
  "settings.refProfileACodecAllow":
    "Разрешённые аудиокодеки (пустое значение означает, что разрешены все)",
  "settings.refProfileACodecBlock": "Заблокированные аудиокодеки",
  "settings.refProfileAtmosPreferred":
    "Профиль отдаёт предпочтение Dolby Atmos, если он доступен",
  "settings.refProfileDVAllowed": "Релизы с Dolby Vision допускаются",
  "settings.refProfileHdrAllowed": "Релизы с HDR допускаются",
  "settings.refProfilePreferRemux":
    "Профиль отдаёт предпочтение полноценным релизам Remux",
  "settings.refProfileAllowBdDisk":
    "Допускаются источники в виде дисков Blu-ray",
  "settings.refProfileAllowUpgrades":
    "Разрешено обновление существующих файлов",
  "settings.refProfilePreferDualAudio":
    "Профиль отдаёт предпочтение релизам с двумя аудиодорожками",
  "settings.refProfileRequiredLangs":
    "Обязательные языки аудио (коды ISO 639-3)",
  "settings.refSectionContext": "— Контекст материала и поиска",
  "settings.refCtxTitleId":
    "Внутренний идентификатор материала в Scryer, если конкретный материал уже известен",
  "settings.refCtxLibraryName":
    "Определённое имя библиотеки, если оценка выполняется в рамках конкретной библиотеки",
  "settings.refCtxMediaType":
    "Оцениваемая категория медиаконтента, обычно movie, series или anime",
  "settings.refCtxCategory":
    "Подсказка категории, используемая при оценке и обработке правил",
  "settings.refCtxOriginalLanguage":
    "Нормализованный язык оригинального названия, если он присутствует в метаданных",
  "settings.refCtxOriginalCountry":
    "Нормализованная страна происхождения, если она присутствует в метаданных",
  "settings.refCtxInferredOriginalAudioLanguage":
    "Нормализованный язык оригинальной звуковой дорожки, определённый для релизов без явной маркировки",
  "settings.refCtxTags":
    "Пользовательские теги материала в нижнем регистре из реестра тегов, который администратор ведёт в настройках; служебные записи с префиксом scryer: исключены",
  "settings.refCtxHasExisting":
    "Истинно, если для этого материала уже существует файл",
  "settings.refCtxExistingScore":
    "Встроенная оценка существующего файла (если он существует)",
  "settings.refCtxSearchMode":
    "Режим оценки. Текущие значения: auto и post_download",
  "settings.refCtxRuntimeMin": "Продолжительность контента в минутах",
  "settings.refCtxIsAnime":
    "Истинно, если материал относится к категории аниме",
  "settings.refCtxIsFiller": "Истинно, если эпизод помечен как филлер",
  "settings.refSectionBuiltinScore":
    "— Результаты встроенного механизма оценки",
  "settings.refBuiltinTotal":
    "Общая оценка, рассчитанная встроенным механизмом",
  "settings.refBuiltinBlocked":
    "Истинно, если встроенные правила заблокировали этот релиз",
  "settings.refBuiltinCodes":
    "Список применённых кодов оценки (например, quality_tier_0)",
  "settings.refSectionFile":
    "— Метаданные фактически проанализированного файла (до загрузки значение равно null)",
  "settings.refFileVideoCodec":
    "Подтверждённый видеокодек, полученный в результате анализа файла",
  "settings.refFileVideoWidth": "Подтверждённая ширина видео в пикселях",
  "settings.refFileVideoHeight": "Подтверждённая высота видео в пикселях",
  "settings.refFileVideoBitrateKbps": "Подтверждённый видеобитрейт в кбит/с",
  "settings.refFileVideoBitDepth": "Подтверждённая глубина цвета видео",
  "settings.refFileVideoHdrFormat":
    "Подтверждённый формат HDR, например Dolby Vision, HDR10, HDR10+ или HLG",
  "settings.refFileDoviProfile":
    "Номер профиля Dolby Vision, если он присутствует",
  "settings.refFileDoviBlCompatId":
    "Идентификатор совместимости базового слоя Dolby Vision, если он присутствует",
  "settings.refFileVideoFrameRate":
    "Подтверждённая частота кадров, определённая при анализе файла",
  "settings.refFileVideoProfile":
    "Подтверждённый профиль кодека, например Main 10 или High",
  "settings.refFileAudioCodec": "Основной подтверждённый аудиокодек",
  "settings.refFileAudioProfile":
    "Основной подтверждённый профиль или расширение аудио, например LC, Dolby TrueHD + Dolby Atmos или DTS-HD MA + DTS:X IMAX",
  "settings.refFileAudioChannels":
    "Основное подтверждённое количество аудиоканалов",
  "settings.refFileAudioBitrateKbps":
    "Основной подтверждённый аудиобитрейт в кбит/с",
  "settings.refFileAudioLanguages":
    "Все обнаруженные языки аудиодорожек в фактическом файле",
  "settings.refFileAudioStreams":
    "Подробные сведения о каждой аудиодорожке; см. input.file.audio_streams[] ниже",
  "settings.refFileSubtitleLanguages":
    "Все обнаруженные языки субтитров в фактическом файле",
  "settings.refFileSubtitleCodecs":
    "Все обнаруженные кодеки субтитров в фактическом файле",
  "settings.refFileSubtitleStreams":
    "Подробные сведения о каждой дорожке субтитров; см. input.file.subtitle_streams[] ниже",
  "settings.refFileHasMultiAudio":
    "Истинно, если в файле присутствует несколько аудиодорожек",
  "settings.refFileDurationSeconds":
    "Подтверждённая продолжительность в секундах",
  "settings.refFileNumChapters":
    "Подтверждённое количество глав, если контейнер поддерживает главы",
  "settings.refFileContainerFormat":
    "Подтверждённый формат контейнера, например matroska или mp4",
  "settings.refSectionAudioStreams": "— Вложенные записи аудиопотоков",
  "settings.refAudioStreamCodec": "Аудиокодек отдельного потока",
  "settings.refAudioStreamProfile":
    "Подтверждённый профиль или метка расширения для отдельного аудиопотока",
  "settings.refAudioStreamChannels":
    "Количество каналов отдельного аудиопотока",
  "settings.refAudioStreamLanguage": "Код языка отдельного аудиопотока",
  "settings.refAudioStreamBitrateKbps":
    "Битрейт отдельного аудиопотока в кбит/с",
  "settings.refSectionSubtitleStreams": "— Вложенные записи потоков субтитров",
  "settings.refSubtitleStreamCodec": "Кодек отдельного потока субтитров",
  "settings.refSubtitleStreamLanguage": "Код языка отдельного потока субтитров",
  "settings.refSubtitleStreamName":
    "Название отдельной дорожки субтитров, если доступно",
  "settings.refSubtitleStreamForced":
    "Истинно, если поток субтитров помечен как принудительный",
  "settings.refSubtitleStreamDefault":
    "Истинно, если поток субтитров помечен как используемый по умолчанию",
  "settings.refSectionBuiltins": "Пользовательские функции Scryer",
  "settings.refBuiltinsIntro":
    "Эти вспомогательные функции доступны в ваших правилах Rego в дополнение ко всем стандартным встроенным функциям OPA.",
  "settings.refFnBlockScore":
    "Возвращает штраф -10000, который можно компенсировать другими баллами",
  "settings.refFnSizeGib":
    "Преобразует байты в ГиБ (например, scryer.size_gib(input.release.size_bytes) > 50)",
  "settings.refFnLangMatches":
    'Сопоставляет коды ISO 639-3 с их псевдонимами (например, scryer.lang_matches("jpn", "ja") возвращает true)',
  "settings.refFnNormalizeSource":
    'Нормализует названия источников (например, "webdl" преобразуется в "WEB-DL")',
  "settings.refFnNormalizeCodec":
    'Нормализует названия кодеков (например, "h264" преобразуется в "H.264")',
  "settings.refSectionSandbox": "Песочница и ограничения",
  "settings.refSandboxIntro":
    "Все пользовательские правила выполняются в изолированной среде Rego со следующими ограничениями:",
  "settings.refSandboxNoIO":
    "Нет доступа к сети или файловой системе — правила не могут выполнять HTTP-запросы, читать файлы или выполнять другие операции ввода-вывода. Стандартные встроенные функции OPA, такие как http.send, недоступны.",
  "settings.refSandboxPkgIsolation":
    "Изоляция пакетов — каждое правило автоматически помещается в собственное пространство имён. Правила не могут читать или изменять данные других правил.",
  "settings.refSandboxReadOnly":
    "Входные данные доступны только для чтения — правила могут только читать документ input. Они не могут изменять состояние приложения или сохранять данные между выполнениями.",
  "settings.refSandboxOutputRestricted":
    "Вывод ограничен score_entry — из каждого правила извлекается только карта score_entry. Все остальные определения и присваивания игнорируются.",
  "settings.refSandboxIntegerOnly":
    "Изменения оценки только целыми числами — значения в score_entry должны быть целыми числами в диапазоне i32. Числа с плавающей точкой отклоняются при проверке (для преобразования используйте round() или ceil()).",
  "settings.refSandboxValidation":
    "Проверка перед сохранением — правила компилируются и тестируются на синтетических входных данных перед сохранением. Некорректные правила отклоняются.",
  "settings.refSandboxErrorIsolation":
    "Изоляция ошибок — если одно правило завершается с ошибкой во время выполнения, все остальные правила продолжают оцениваться в обычном режиме. Ошибки записываются в журнал, но не прерывают процесс оценки.",
  "settings.refSectionOutput": "Формат вывода правила",
  "settings.refOutputIntro":
    "Правила должны определять правило score_entry, которое возвращает карту со строковыми ключами (кодами оценки) и целочисленными значениями (изменениями баллов). Положительные значения повышают оценку релиза, отрицательные — снижают её, а scryer.block_score() добавляет штраф -10000, который можно компенсировать. Решение принимается после суммирования всех баллов; обязательные требования профиля остаются в силе. Правила, основанные на файлах, до загрузки автоматически ничего не делают, поскольку input.file имеет значение null до этапа оценки после загрузки.",
  "settings.subtitles": "Субтитры",
  "settings.subtitlesDescription":
    "Настройка языков субтитров, порогов сопоставления, поведения синхронизации и подключений к провайдерам.",
  "settings.subtitlesSaved": "Настройки субтитров сохранены.",
  "settings.subtitleProviders": "Провайдеры субтитров",
  "settings.existingSubtitleProviders": "Настроенные провайдеры субтитров",
  "settings.subtitleProviderType": "Провайдер",
  "settings.subtitleProviderStatus": "Статус",
  "settings.subtitleProviderFacets": "Типы контента",
  "settings.subtitleProviderFacetsHelp":
    "Scryer выполняет поиск у этого провайдера только для выбранных типов контента. Рекомендации плагина задают начальные значения по умолчанию, но не ограничивают ваш выбор.",
  "settings.subtitleProviderEmpty": "Провайдеры субтитров пока не настроены.",
  "settings.subtitleProviderCreate": "Добавить провайдера субтитров",
  "settings.subtitleProviderCreateNew": "Создать нового провайдера субтитров",
  "settings.subtitleProviderEdit": "Изменить провайдера субтитров",
  "settings.subtitleProviderConfirmDiscardTitle":
    "Отменить текущие изменения провайдера субтитров?",
  "settings.subtitleProviderConfirmDiscardDescription":
    "Текущие изменения провайдера субтитров будут потеряны.",
  "settings.subtitleProviderNamePlaceholder": "OpenSubtitles",
  "settings.subtitleProviderCreated": "Провайдер субтитров создан.",
  "settings.subtitleProviderUpdated": "Провайдер субтитров обновлён.",
  "settings.subtitleProviderDeleted": "{{name}} удалён.",
  "settings.subtitleProviderNoActivity": "Активности пока нет",
  "settings.subtitleProviderDisabledUntil": "Отключён до {{time}}",
  "settings.subtitleProviderLastError": "Последняя ошибка: {{time}}",
  "settings.subtitleProviderUnknownType":
    "Этот тип провайдера установлен, но схема его полей пока недоступна.",
  "settings.subtitleProviderSecretStored":
    "Сохранённое значение останется без изменений, если его не заменить",
  "settings.subtitleProviderBindingManaged": "Предоставляется хостом",
  "settings.subtitleProviderBindingAvailable": "Доступно",
  "settings.subtitleProviderBindingMissing": "Отсутствует",
  "settings.sub.enabled": "Включить управление субтитрами",
  "settings.sub.credentials": "Учётные данные OpenSubtitles",
  "settings.sub.username": "Имя пользователя",
  "settings.sub.usernameEmailWarning":
    "Используйте имя пользователя OpenSubtitles, а не адрес электронной почты.",
  "settings.sub.password": "Пароль",
  "settings.sub.apiKey": "API-ключ",
  "settings.sub.apiKeyRequiredTitle": "Сервис субтитров недоступен",
  "settings.sub.apiKeyRequiredBody":
    "Поиск и загрузка субтитров будут отключены, пока сервис субтитров не станет доступен.",
  "settings.sub.credentialsRequiredTitle":
    "Требуются учётные данные для субтитров",
  "settings.sub.credentialsRequiredBody":
    "Укажите здесь имя пользователя и пароль для сервиса субтитров перед поиском или загрузкой субтитров.",
  "settings.sub.credentialsVerified":
    "Учётные данные для субтитров подтверждены",
  "settings.sub.languages": "Языки субтитров",
  "settings.sub.languagesHelp":
    "Коды ISO 639-2 через запятую (например, eng, spa, fre, deu, jpn)",
  "settings.sub.languagePickerAriaLabel": "Выбрать языки субтитров",
  "settings.sub.languagePickerSelect": "Выберите языки...",
  "settings.sub.languagePickerSearch": "Поиск языков...",
  "settings.sub.languagePickerEmpty": "Языки не найдены",
  "settings.sub.languagePickerRemove": "Удалить {{language}}",
  "settings.sub.hiPreference": "Для слабослышащих",
  "settings.sub.forcedOnly": "Только принудительные субтитры",
  "settings.sub.minScoreSeries": "Минимальный процент совпадения для сериалов",
  "settings.sub.minScoreMovie": "Минимальный процент совпадения для фильмов",
  "settings.sub.minScoreHelp":
    "Субтитры с меньшим процентом совпадения будут пропущены. Для сериалов требуется более высокий порог, поскольку совпадение на уровне эпизодов должно быть точным.",
  "settings.sub.searchInterval": "Интервал поиска (часы)",
  "settings.sub.autoDownload": "Автоматически загружать при импорте",
  "settings.sub.excludeAi": "Исключать субтитры, переведённые ИИ",
  "settings.sub.excludeMachine": "Исключать субтитры с машинным переводом",
  "settings.sub.syncEnabled":
    "Автоматически исправлять синхронизацию субтитров",
  "settings.sub.syncEnabledHelp":
    "Использует анализ аудио для исправления смещения времени в загруженных субтитрах.",
  "settings.sub.syncThresholdSeries":
    "Автокоррекция при этом проценте или ниже (сериалы)",
  "settings.sub.syncThresholdMovie":
    "Автокоррекция при этом проценте или ниже (фильмы)",
  "settings.sub.syncThresholdHelp":
    "Субтитры с низким процентом совпадения чаще имеют проблемы с синхронизацией. Субтитры с более высоким процентом совпадения считаются хорошо синхронизированными.",
  "settings.sub.syncMaxOffset": "Максимальная коррекция (секунды)",
  "settings.sub.syncMaxOffsetHelp":
    "Пропускать изменения синхронизации, превышающие это значение, чтобы не испортить субтитры, которые уже достаточно хорошо синхронизированы.",
  "settings.sub.syncPluginRequiredTitle":
    "Требуется расширенная синхронизация субтитров",
  "settings.sub.syncPluginRequiredDescription":
    "Установите {{plugin}}, чтобы разблокировать настройки автоматической коррекции синхронизации.",
  "settings.sub.syncPluginLoadingCatalog": "Проверка каталога плагинов...",
  "settings.sub.syncPluginInstall": "Установить плагин",
  "settings.sub.syncPluginInstalling": "Установка...",
  "settings.sub.syncPluginUnavailable":
    "Расширенная синхронизация субтитров недоступна в каталоге плагинов.",
  "settings.sub.syncPluginBlocked":
    "Не удалось установить расширенную синхронизацию субтитров: {{reason}}",
  "settings.sub.syncPluginDisabled":
    "Расширенная синхронизация субтитров установлена, но отключена. Включите её в разделе «Плагины», чтобы использовать коррекцию синхронизации.",
  "settings.sub.syncPluginInstalled": "{{plugin}} установлен",
  "settings.sub.syncPluginLoadFailed": "Не удалось загрузить каталог плагинов.",
  "settings.sub.syncPluginInstallFailed":
    "Не удалось установить {{plugin}}: {{error}}",
  "settings.sub.serviceStatus": "Сервис субтитров",
  "settings.sub.serviceStatusAvailable": "Доступен",
  "settings.sub.serviceStatusUnavailable": "Недоступен",
  "settings.postProcessing": "Постобработка",
  "settings.postProcessingSaved": "Скрипт постобработки сохранён.",
  "settings.pp.title": "Скрипты постобработки",
  "settings.pp.description":
    "Скрипты запускаются автоматически после импорта медиафайлов. Блокирующие скрипты выполняются по порядку приоритета. Скрипты без ожидания завершения запускаются параллельно и не задерживают процесс импорта.",
  "settings.pp.noScripts": "Скрипты постобработки не настроены.",
  "settings.pp.name": "Название скрипта",
  "settings.pp.namePlaceholder": "например, уведомить Plex",
  "settings.pp.descriptionLabel": "Описание",
  "settings.pp.descriptionPlaceholder": "Что делает этот скрипт?",
  "settings.pp.scriptType": "Тип скрипта",
  "settings.pp.inline": "Встроенная shell-команда",
  "settings.pp.filePath": "Путь к файлу",
  "settings.pp.inlineHelp":
    "Введите shell-команду для выполнения через командную оболочку сервера.",
  "settings.pp.filePathHelp":
    "Абсолютный путь к исполняемому скрипту на сервере. Аргументы, перенаправление и подстановка командной оболочки не применяются.",
  "settings.pp.blocking": "Блокирующий",
  "settings.pp.fireAndForget": "Запустить без ожидания",
  "settings.pp.blockingHelp":
    "Выполняется в порядке приоритета. Следующий скрипт ожидает завершения текущего.",
  "settings.pp.fireAndForgetHelp":
    "Запускается в фоне, не блокируя импорт. Вывод при этом всё равно может сохраняться, если эта функция включена.",
  "settings.pp.executionMode": "Режим выполнения",
  "settings.pp.timeout": "Тайм-аут (секунды)",
  "settings.pp.timeoutLabel": "Тайм-аут",
  "settings.pp.priority": "Приоритет",
  "settings.pp.priorityHelp": "Меньшее число выполняется раньше.",
  "settings.pp.facets": "Применяется к",
  "settings.pp.debug": "Сохранять вывод скрипта",
  "settings.pp.debugHelp":
    "Если включено, stdout и stderr сохраняются для каждого запуска, включая фоновые. Полезно для диагностики.",
  "settings.pp.runHistory": "История запусков",
  "settings.pp.noRuns": "Запусков пока не было.",
  "settings.pp.outputNotCaptured":
    "Вывод не сохранён. Включите режим отладки для этого скрипта, чтобы увидеть вывод.",
  "settings.pp.envHeading": "Переменные окружения",
  "settings.pp.envDescription":
    "Скрипты получают JSON-данные в SCRYER_METADATA, а также показанные ниже вспомогательные переменные. Значения могут быть null, если Scryer не располагает соответствующими данными.",
  "settings.pp.created": "Скрипт создан.",
  "settings.pp.updated": "Скрипт обновлён.",
  "settings.pp.deleted": "Скрипт удалён.",
  "settings.pp.toggled": "Скрипт {{state}}.",
  "settings.pp.createNewScript": "Создать новый скрипт",
  "settings.pp.confirmDiscardTitle":
    "Отменить текущие изменения скрипта постобработки?",
  "settings.pp.confirmDiscardDescription":
    "Текущие изменения скрипта постобработки будут потеряны.",
  "settings.pp.inlineShellConfirmTitle": "Разрешить встроенный shell-скрипт?",
  "settings.pp.inlineShellConfirmDescription":
    "Встроенные скрипты постобработки выполняются через командную оболочку сервера после импорта. Сохраняйте только те скрипты, которые вы проверили и которым доверяете.",
  "settings.pp.inlineShellConfirm": "Разрешить встроенный shell-скрипт",
  "settings.plugins": "Плагины",
  "settings.scripts": "Скрипты",
  "settings.pluginsSection": "Управление плагинами",
  "settings.pluginRuntimeMemoryEstimate":
    "Оценочное использование памяти плагинами во время выполнения",
  "settings.pluginsRefresh": "Обновить каталог",
  "settings.pluginsUpdateAll": "Обновить все",
  "settings.pluginsUpdatingAll": "Обновление всех…",
  "settings.host": "Хост",
  "settings.port": "Порт",
  "settings.baseUrl": "Базовый URL",
  "settings.pluginsInstalled": "Установленные",
  "settings.pluginsAvailable": "Доступные",
  "settings.pluginsNoInstalled": "Установленные плагины отсутствуют.",
  "settings.pluginsNoAvailable":
    "Нет плагинов, соответствующих текущему фильтру.",
  "settings.pluginsNoPlugins":
    "Плагины не найдены. Обновите каталог, чтобы проверить наличие доступных плагинов.",
  "settings.pluginAllCategories": "Все категории",
  "settings.pluginCategoryIndexer": "Индексатор",
  "settings.pluginCategoryUsenetIndexer": "Индексатор Usenet",
  "settings.pluginCategoryTorrentIndexer": "Торрент-индексатор",
  "settings.pluginCategoryDownloadClient": "Клиент загрузки",
  "settings.pluginCategoryArchiveExtractor": "Распаковка архивов",
  "settings.pluginCategoryNotification": "Уведомления",
  "settings.pluginCategorySubtitleProvider": "Провайдер субтитров",
  "settings.pluginOfficialOnly": "Только официальные",
  "settings.pluginInstall": "Установить",
  "settings.pluginInstalling": "Установка…",
  "settings.pluginInstallDownloading": "Загрузка",
  "settings.pluginInstallVerifying": "Проверка",
  "settings.pluginInstallInstalling": "Установка",
  "settings.pluginUninstall": "Удалить",
  "settings.pluginRevertToBundled": "Вернуться к встроенной версии",
  "settings.pluginRevert": "Вернуть",
  "settings.pluginUninstallWarning":
    "Удаление этого плагина приведёт к удалению всех связанных с ним конфигураций.",
  "settings.pluginRevertToBundledWarning":
    "Загруженное переопределение для «{{name}}» будет удалено, после чего будет восстановлена встроенная версия плагина.",
  "settings.pluginBuiltin": "Встроенный",
  "settings.pluginInstalled": "Установлен",
  "settings.pluginNotInstalled": "Не установлен",
  "settings.pluginOfficial": "Официальный",
  "settings.pluginVerifiedCommunity": "Проверенный сообществом",
  "settings.pluginUnverified": "Непроверенный",
  "settings.pluginBeta": "Бета",
  "settings.pluginDeprecated": "Устаревший",
  "settings.pluginSource": "Источник",
  "settings.pluginDocs": "Документация",
  "settings.pluginInstallManually": "Установить вручную",
  "settings.pluginManualUploadTitle": "Загрузка локального файла",
  "settings.pluginManualUploadHelp":
    "Загрузите локальную сборку плагина для быстрого тестирования. Scryer принимает необработанные файлы .wasm и пакеты .wasm.zst.",
  "settings.pluginManualUploadSelect": "Выбрать файл плагина",
  "settings.pluginManualUploadInstall": "Проверить и установить файл",
  "settings.pluginManualUploadNoFile": "Файл плагина не выбран.",
  "settings.pluginManualUploadFormats":
    "Поддерживаемые форматы: .wasm и .wasm.zst",
  "settings.pluginManualFileUnsupported":
    "Для ручной загрузки плагина необходимо выбрать файл .wasm или .wasm.zst.",
  "settings.pluginManualUploadConfirmTitle":
    "Установить локальный файл плагина?",
  "settings.pluginManualUploadConfirmDescription":
    "Вы собираетесь запустить непроверенный локальный код плагина внутри Scryer. Устанавливайте только файлы, которые вы создали самостоятельно или которым явно доверяете.",
  "settings.pluginManualUploadConfirmCheckbox":
    "Я понимаю, что этот плагин будет выполнять пользовательский код с разрешениями, указанными в его манифесте.",
  "settings.pluginManualUploadConfirmAction": "Установить локальный плагин",
  "settings.pluginManualRepoTitle": "Установка из репозитория GitHub",
  "settings.pluginManualRepoHelp":
    "Установите непроверенный плагин напрямую из репозитория GitHub, публикующего каталог плагинов в формате Scryer.",
  "settings.pluginManualRepoUrl": "Репозиторий плагина на GitHub",
  "settings.pluginInspectManual": "Проверить репозиторий",
  "settings.pluginOverride": "Переопределение",
  "settings.pluginVersion": "v{{version}}",
  "settings.pluginBytes": "WASM {{bytes}}",
  "settings.pluginUpdateAvailable": "Обновление: v{{version}}",
  "settings.pluginUpgrade": "Обновить до v{{version}}",
  "settings.pluginOptimizedBuildAvailable": "Доступна оптимизированная сборка",
  "settings.pluginInstallOptimizedBuild": "Установить оптимизированную сборку",
  "settings.pluginNoCompatibleRelease":
    "Нет совместимой версии для этой версии Scryer",
  "settings.pluginNewerReleaseRequiresNewerScryer":
    "Для более новой версии плагина требуется более новая версия Scryer",
  "settings.pluginNewerReleaseRequiresNewerScryerVersion":
    "Для версии плагина v{{version}} требуется более новая версия Scryer",
  "status.pluginInstalled": "Плагин «{{name}}» установлен.",
  "status.pluginInstallAlreadyInProgress":
    "Плагин «{{name}}» уже устанавливается или обновляется.",
  "status.pluginInstallFailedChecksumMismatch":
    "Не удалось установить плагин «{{name}}»: опубликованный файл больше не соответствует контрольной сумме из реестра. Обновите реестр или повторите попытку позже.",
  "status.pluginInstallFailedSdkMetadataMismatch":
    "Не удалось установить плагин «{{name}}»: опубликованный файл не соответствует данным о совместимости SDK из реестра. Обновите реестр или повторите попытку позже.",
  "status.pluginInstallFailedWithReason":
    "Не удалось установить плагин «{{name}}»: {{reason}}",
  "status.pluginRevertedToBundled":
    "Плагин «{{name}}» возвращён к встроенной версии.",
  "status.pluginUninstalled": "Плагин «{{name}}» удалён.",
  "status.pluginUpgraded": "Плагин «{{name}}» обновлён до версии v{{version}}.",
  "status.pluginsUpgradeQueued":
    "Обновление поставлено в очередь для {{count}} плагинов.",
  "status.pluginToggled": "Плагин «{{name}}» {{state}}.",
  "status.registryRefreshed": "Реестр плагинов обновлён.",
  "status.catalogRefreshed": "Каталог плагинов обновлён.",
  "form.namePlaceholder": "Название",
  "form.indexerNamePlaceholder": "Мой индексатор Newznab",
  "form.providerTypePlaceholder": "Тип провайдера",
  "form.providerTypePlaceholderValue": "nzbgeek",
  "form.subtitleProviderValidation":
    "Необходимо указать название и тип провайдера.",
  "form.baseUrlPlaceholder": "Базовый URL",
  "form.baseUrlPlaceholderValue": "https://api.nzbgeek.info/api",
  "form.apiKeyPlaceholder": "API-ключ",
  "form.apiKeyInputPlaceholder": "api-ключ",
  "form.rateLimitSeconds": "Ограничение запросов (секунды)",
  "form.rateLimitBurst": "Пакетный лимит запросов",
  "form.defaultPlaceholder": "По умолчанию из сервиса",
  "form.usernamePlaceholder": "имя пользователя",
  "form.passwordPlaceholder": "пароль",
  "form.newPasswordPlaceholder": "новый пароль",
  "form.indexerValidation":
    "Необходимо указать название, тип провайдера и базовый URL.",
  "form.indexerApiKeyRequired": "Для этого индексатора требуется API-ключ.",
  "form.indexerRateLimitValidation":
    "Значения ограничения запросов должны быть числовыми.",
  "form.rateLimitPlaceholder": "По умолчанию из сервиса",
  "form.enabled": "Включено",
  "qualityProfile.existingProfiles": "Существующие профили качества",
  "qualityProfile.noProfilesFound": "Профили не найдены.",
  "qualityProfile.createProfile": "Создать профиль качества",
  "qualityProfile.createNewProfile": "Создать новый профиль качества",
  "qualityProfile.defaultProfileName": "По умолчанию",
  "qualityProfile.idGeneratedDescription":
    "Идентификатор профиля будет создан на основе названия.",
  "qualityProfile.createProfileDescription":
    "Названия профилей можно изменять после создания; идентификаторы генерируются автоматически и должны оставаться уникальными.",
  "qualityProfile.editProfile": "Редактировать профиль качества",
  "qualityProfile.confirmCreateFromEditTitle":
    "Отменить текущие изменения профиля качества?",
  "qualityProfile.confirmCreateFromEditDescription":
    "Текущие изменения профиля качества будут потеряны и заменены новой пустой формой профиля.",
  "qualityProfile.confirmDiscardTitle":
    "Отменить текущие изменения профиля качества?",
  "qualityProfile.confirmDiscardDescription":
    "Текущие изменения профиля качества будут потеряны.",
  "qualityProfile.profileNameLabel": "Название профиля",
  "qualityProfile.qualityTiersAndArchival":
    "Уровни качества и архивное качество",
  "qualityProfile.allowedQualityTiers": "Разрешённые уровни качества (выбрано)",
  "qualityProfile.noQualityTiersSelected": "Уровни качества не выбраны.",
  "qualityProfile.removeQualityTier": "Удалить уровень качества {{value}}",
  "qualityProfile.addQualityTier": "Добавить {{value}} в выбранные",
  "qualityProfile.availableQualityTiers": "Доступные уровни качества",
  "qualityProfile.allQualityTiersSelected":
    "Выбраны все известные уровни качества.",
  "qualityProfile.archivalQuality": "Архивное качество",
  "qualityProfile.archivalQualityInfo":
    "Уровень качества, который будет сохраняться для долгосрочного хранения. Scryer может сначала загрузить версию более высокого качества, если архивная версия ещё недоступна, а затем заменить её, когда будет найден архивный релиз.",
  "qualityProfile.qualityTiers": "Уровни качества",
  "qualityProfile.otherOptions": "Другие параметры",
  "qualityProfile.allowHdr": "HDR",
  "qualityProfile.detectedHdrAllowed": "Разрешить HDR",
  "qualityProfile.detectedHdrAllowedInfo":
    "Разрешить релизы, определённые как HDR (HDR10, HLG, Dolby Vision и т. д.). Обратите внимание: многие названия релизов не содержат явного указания HDR, поэтому это не гарантирует фильтрацию всех HDR-релизов.",
  "qualityProfile.allowUnknownQuality": "Разрешить неизвестное качество",
  "qualityProfile.allowUnknownQualityInfo":
    "Разрешить релизы, качество которых не удалось определить с помощью правил анализа. Это может помочь с нестандартными кодировками, но может привести к появлению совпадений с более низкой степенью достоверности.",
  "qualityProfile.dolbyVisionAllowed": "Разрешить Dolby Vision",
  "qualityProfile.dolbyVisionInfo":
    "Разрешить релизы с Dolby Vision. Некоторые дисплеи не могут корректно воспроизводить контент Dolby Vision, поэтому этот параметр подходит не всем.",
  "qualityProfile.preferRemux": "Предпочитать Remux",
  "qualityProfile.preferRemuxInfo":
    "Remux — это полноразмерные копии физических носителей без потери качества. Эта функция особенно полезна при использовании профиля Audiophile.",
  "qualityProfile.allowBdDisk": "Разрешить BD-диски",
  "qualityProfile.allowBdDiskInfo":
    "Разрешить релизы в формате BD/образов дисков (ISO и релизы в формате Blu-ray Disc).",
  "qualityProfile.allowDv": "Dolby Vision",
  "qualityProfile.allowUpgrades": "Разрешить обновления",
  "qualityProfile.allowUpgradesInfo":
    "Разрешить замену существующих копий более качественными версиями, но не выше выбранного архивного качества.",
  "qualityProfile.sourceAllowlist": "Источники",
  "qualityProfile.sourceBlocklist": "Чёрный список источников",
  "qualityProfile.videoCodecAllowlist": "Видеокодеки",
  "qualityProfile.videoCodecBlocklist": "Чёрный список видеокодеков",
  "qualityProfile.audioCodecAllowlist": "Аудиокодеки",
  "qualityProfile.audioCodecBlocklist": "Чёрный список аудиокодеков",
  "qualityProfile.sourceAllowlistDefault":
    "По умолчанию разрешены все источники.",
  "qualityProfile.sourceAllowlistInfo":
    "Разрешать только релизы, соответствующие одному из выбранных источников (например, WEB-DL или HDTV).",
  "qualityProfile.sourceBlocklistDefault":
    "По умолчанию источники не блокируются.",
  "qualityProfile.sourceBlocklistInfo":
    "Явно блокировать релизы из этих источников, даже если они соответствуют другим критериям.",
  "qualityProfile.videoCodecAllowlistDefault":
    "По умолчанию разрешены все видеокодеки.",
  "qualityProfile.videoCodecAllowlistInfo":
    "Разрешать только релизы, закодированные с использованием одного из выбранных видеокодеков.",
  "qualityProfile.videoCodecBlocklistDefault":
    "По умолчанию видеокодеки не блокируются.",
  "qualityProfile.videoCodecBlocklistInfo":
    "Всегда отклонять релизы, использующие эти видеокодеки.",
  "qualityProfile.audioCodecAllowlistDefault":
    "По умолчанию разрешены все аудиокодеки.",
  "qualityProfile.audioCodecAllowlistInfo":
    "Разрешать только релизы с соответствующими аудиокодеками.",
  "qualityProfile.audioCodecBlocklistDefault":
    "По умолчанию аудиокодеки не блокируются.",
  "qualityProfile.audioCodecBlocklistInfo":
    "Отклонять релизы, содержащие любой аудиокодек из этого списка.",
  "qualityProfile.selectedItemsLabel": "Выбрано",
  "qualityProfile.availableItemsLabel": "Доступно",
  "qualityProfile.noSelectedItems": "Ничего не выбрано.",
  "qualityProfile.noAvailableItems": "Нет подходящих элементов.",
  "qualityProfile.aboutSection": "О профиле {{title}}",
  "qualityProfile.removeItem": "Удалить {{value}} из выбранных",
  "qualityProfile.addItem": "Добавить {{value}} в выбранные",
  "qualityProfile.useDefaultQualityFallback":
    "Использовать качество по умолчанию",
  "qualityProfile.defaultCategoryProfiles": "Профили категорий по умолчанию",
  "qualityProfile.scoringSection": "Оценка",
  "qualityProfile.scoringSectionInfo":
    "Управляйте способом ранжирования релизов. Выберите готовый профиль и при необходимости переопределите отдельные параметры оценки.",
  "qualityProfile.scoringAndPreferences": "Оценка и предпочтения",
  "qualityProfile.scoringAndPreferencesInfo":
    "Управляйте ранжированием и фильтрацией релизов. Настройте профиль оценки, предпочтения и правила обновления.",
  "qualityProfile.facetPersonaOverrides": "Профили для отдельных категорий",
  "qualityProfile.facetPersonaOverridesInfo":
    "Переопределите профиль оценки для определённых типов медиа. Например, используйте Audiophile для аниме, а Efficient — для фильмов.",
  "qualityProfile.facetPersonaUseDefault": "Использовать значение по умолчанию",
  "qualityProfile.scoringPersona": "Профиль оценки",
  "qualityProfile.scoringPersonaInfo":
    "Готовые наборы весов оценки, отдающие приоритет различным характеристикам релизов. Для большинства пользователей рекомендуется профиль Balanced.",
  "qualityProfile.scoringPersonaMovedInfo":
    "Профиль оценки теперь находится в настройках качества, а не внутри сохранённых профилей качества. Переопределения оценки профиля по-прежнему применяются поверх текущего профиля.",
  "qualityProfile.personaBalanced": "Сбалансированный",
  "qualityProfile.personaAudiophile": "Аудиофил",
  "qualityProfile.personaEfficient": "Эффективный",
  "qualityProfile.personaCompatible": "Совместимый",
  "qualityProfile.scoringOverrides": "Переопределение параметров оценки",
  "qualityProfile.scoringOverridesInfo":
    "Переопределите отдельные параметры оценки независимо от выбранного профиля. Оставьте значение «По умолчанию», чтобы использовать настройки профиля.",
  "qualityProfile.overrideDefault": "По умолчанию",
  "qualityProfile.overrideAllowX265Non4k": "Разрешить x265 ниже 4K",
  "qualityProfile.overrideAllowX265Non4kInfo":
    "Разрешить релизы, закодированные с использованием x265/HEVC, при разрешении ниже 4K. Некоторые устройства плохо работают с x265 при более низких разрешениях.",
  "qualityProfile.overrideBlockDvNoFallback":
    "Блокировать DV без резервного слоя",
  "qualityProfile.overrideBlockDvNoFallbackInfo":
    "Блокировать релизы Dolby Vision без резервного слоя HDR10, которые на дисплеях без поддержки DV могут отображаться в зелёных или фиолетовых оттенках.",
  "qualityProfile.overridePreferCompact": "Предпочитать компактные кодировки",
  "qualityProfile.overridePreferCompactInfo":
    "Отдавать предпочтение меньшим по размеру и более эффективным с точки зрения пропускной способности кодировкам. Полезно при ограниченном объёме хранилища или потоковой передаче через медленное соединение.",
  "qualityProfile.overridePreferLossless": "Предпочитать аудио без потерь",
  "qualityProfile.overridePreferLosslessInfo":
    "Начислять дополнительные баллы релизам с аудиодорожками без потерь (TrueHD, FLAC, DTS-HD MA).",
  "qualityProfile.overrideBlockUpscaled": "Блокировать ИИ-апскейл",
  "qualityProfile.overrideBlockUpscaledInfo":
    "Блокировать релизы, масштабированные с помощью ИИ. Такие релизы часто содержат артефакты и не являются материалом с исходным разрешением.",
  "qualityProfile.cutoffTier": "Пороговый уровень",
  "qualityProfile.cutoffTierInfo":
    "Прекратить обновление после получения этого уровня качества или выше. Это предотвращает ненужные повторные загрузки.",
  "qualityProfile.cutoffNone": "Без ограничений",
  "qualityProfile.minScoreToGrab": "Минимальный балл для загрузки",
  "qualityProfile.minScoreToGrabInfo":
    "Загружать только релизы, оценка которых равна или превышает этот порог. Оставьте поле пустым, чтобы отключить ограничение.",
  "qualityProfile.minScorePlaceholder": "Без ограничений",
  "qualityProfile.confirmDeleteTitle": "Удаление профиля качества",
  "qualityProfile.confirmDeleteDescription":
    "Вы уверены, что хотите удалить профиль качества «{{name}}»? Это действие нельзя отменить.",
  "qualityProfile.deleteDisabledInUse":
    "Этот профиль используется. Перед удалением удалите его из всех заголовков и настроек категорий по умолчанию.",
  "episode.details": "Подробности",
  "episode.search": "Поиск",
  "episode.blocklist": "Список блокировок",
  "episode.overview": "Обзор",
  "episode.fileOnDisk": "Файл на диске",
  "episode.airDate": "Дата выхода",
  "episode.numberLabel": "Эпизод",
  "episode.quality": "Качество",
  "episode.missing": "Отсутствует",
  "episode.noFile": "Этот эпизод ещё не был загружен.",
  "episode.noBlockedReleases": "Для этого эпизода нет заблокированных релизов.",
  "episode.untitledRelease": "Релиз без названия",
  "episode.filler": "Филлер",
  "episode.recap": "Рекап",
  "episode.absoluteNumber": "Абс. №{{number}}",
  "episode.titleTba": "Будет объявлено",
  "episode.special": "Спецвыпуск",
  "episode.ova": "OVA",
  "episode.ona": "ONA",
  "episode.alternate": "Альтернативный",
  "episode.multiAudio": "Две аудиодорожки",
  "seasonSection.noEpisodeRecords": "Для этого сезона нет записей об эпизодах.",
  "mediaFile.pendingScan": "Ожидает сканирования",
  "mediaFile.scanFailed": "Ошибка сканирования",
  "mediaFile.score": "Оценка: {{score}}",
  "mediaFile.primary": "Основной",
  "mediaFile.additional": "Дополнительный",
  "mediaFile.makePrimary": "Сделать основным",
  "mediaFile.video": "Видео",
  "mediaFile.audioTracks": "Аудио",
  "mediaFile.audioCount": "{{count}} аудиодорожек",
  "mediaFile.subtitles": "Субтитры",
  "mediaFile.subtitleCount": "{{count}} субтитров",
  "mediaFile.delete": "Удалить файл",
  "mediaFile.pathNotRecorded": "Путь не сохранён",
  "subtitle.search": "Поиск субтитров",
  "subtitle.searching": "Поиск...",
  "subtitle.download": "Скачать",
  "subtitle.downloading": "Загрузка...",
  "subtitle.noResults": "Субтитры не найдены",
  "subtitle.score": "Оценка",
  "subtitle.scoreWithValue": "Оценка: {{score}}",
  "subtitle.hashMatch": "Совпадение по хэшу",
  "subtitle.hearingImpaired": "Для слабослышащих",
  "subtitle.forced": "Принудительные",
  "subtitle.aiTranslated": "ИИ",
  "subtitle.machineTranslated": "Машинный перевод",
  "subtitle.subtitles": "Субтитры",
  "subtitle.embedded": "Встроенные",
  "subtitle.external": "Внешние",
  "subtitle.manualSearch": "Ручной поиск",
  "subtitle.providersRequiredTitle": "Требуется провайдер субтитров",
  "subtitle.providersRequiredBody":
    "Добавьте и включите хотя бы одного провайдера субтитров перед поиском.",
  "subtitle.providersRequiredAction": "Открыть настройки провайдеров субтитров",
  "subtitle.providerUnavailableTitle": "Сервис субтитров недоступен",
  "subtitle.providerUnavailableBody":
    "Сервис субтитров в данный момент недоступен для этого файла.",
  "subtitle.credentialsRequiredTitle": "Требуются учётные данные",
  "subtitle.credentialsRequiredBody":
    "Настройте имя пользователя и пароль в разделе «Настройки → Субтитры», прежде чем выполнять поиск субтитров для этого файла.",
  "subtitle.credentialsRequiredAction": "Открыть настройки субтитров",
  "subtitle.blocklist": "Список блокировок",
  "subtitle.blocklisted": "Субтитры добавлены в список блокировок",
  "subtitle.deleted": "Субтитры удалены",
  "subtitle.onDisk": "На диске",
  "subtitle.releaseInfo": "Релиз",
  "subtitle.provider": "Провайдер",
  "subtitle.language": "Язык",
  "subtitle.downloadCount": "Количество загрузок",
  "subtitle.selectLanguage": "Выберите язык",
  "subtitle.flags": "Метки",
  "subtitle.noneDownloaded": "Внешние субтитры не загружены.",
  "subtitle.synced": "Синхронизированы",
  "anime.malLink": "MyAnimeList",
  "anime.anilistLink": "AniList",
  "anime.anidbLink": "AniDB",
  "anime.fillerCount": "{{count}} филлерных эпизодов",
  "anime.metadataPanel": "Информация об аниме",
  "anime.malScore": "Оценка MAL",
  "anime.mediaType": "Тип",
  "anime.status": "Статус",
  "anime.fillerSummary":
    "{{fillerCount}} из {{totalCount}} эпизодов являются филлерами ({{percent}}%)",
  "anime.noFiller": "Филлерные эпизоды отсутствуют",
  "settings.fillerPolicyLabel": "Политика обработки филлерных эпизодов",
  "settings.fillerPolicyDownloadAll": "Загружать все эпизоды",
  "settings.fillerPolicySkipFiller": "Пропускать филлерные эпизоды",
  "settings.recapPolicyLabel": "Политика обработки эпизодов-рекапов",
  "settings.recapPolicyDownloadAll": "Загружать все эпизоды",
  "settings.recapPolicySkipRecap": "Пропускать эпизоды-рекапы",
  "settings.monitorSpecialsLabel": "Отслеживать спецвыпуски и OVA",
  "settings.monitorSpecialsDescription":
    "Автоматически отслеживать специальные эпизоды и OVA при добавлении аниме",
  "settings.interSeasonMoviesLabel": "Межсезонные фильмы",
  "settings.interSeasonMoviesDescription":
    "Создавать сюжетные записи для канонических фильмов между сезонами",
  "settings.monitorFillerMoviesLabel": "Отслеживать фильмы-филлеры",
  "settings.monitorFillerMoviesDescription":
    "Включать фильмы, классифицированные как филлеры, в очередь загрузки",
  "settings.importModeLabel": "Режим импорта",
  "settings.importModeDescription":
    "Выберите, должен ли Scryer сохранять исходный файл после импорта в библиотеку или удалять его.",
  "settings.importModeHardlinkCopy": "Жёсткая ссылка / Копирование",
  "settings.importModeMove": "Перемещение",
  "settings.setPermissionsLinuxLabel": "Устанавливать права Unix",
  "settings.setPermissionsLinuxDescription":
    "Применять настроенные права доступа и группу при импорте в Unix. Числовые значения групп интерпретируются как GID.",
  "settings.fileChmodLabel": "Права доступа для файлов",
  "settings.folderChmodLabel": "Права доступа для папок",
  "settings.chownGroupLabel": "Группа-владелец",
  "settings.chmodValidation":
    "Используйте 3 или 4 цифры в восьмеричном формате.",
  "settings.fileChmodDeriveFromFolder": "Наследовать права доступа от папки",
  "settings.chmodPresetOwnerWriteEveryoneRead":
    "Владелец - запись, остальные - чтение",
  "settings.chmodPresetOwnerGroupWriteOtherRead":
    "Владелец и группа - запись, остальные - чтение",
  "settings.chmodPresetOwnerGroupWrite": "Владелец и группа - запись",
  "settings.chmodPresetOwnerWriteGroupRead":
    "Владелец - запись, группа - чтение",
  "settings.chmodPresetEveryoneWrite": "Запись для всех",
  "settings.chmodPresetCustom": "Пользовательские настройки",
  "settings.sidecarFilesTitle": "Дополнительные файлы",
  "settings.identityStorageTitle": "Идентификация и хранилище",
  "settings.mediaProfilesTitle": "Профили медиа",
  "settings.mediaProfilesHelp":
    "Оставьте значение «Наследовать настройки каталога», чтобы использовать параметры каталога, или задайте собственные параметры для каждой библиотеки. Итоговое значение отображается под каждым параметром.",
  "settings.importBehaviorTitle": "Поведение при импорте",
  "settings.libraryUnsavedChanges": "Несохранённые изменения",
  "settings.nfoWriteOnImportLabel": "Создавать NFO-файлы при импорте",
  "settings.nfoWriteOnImportDescription":
    "Создавать файлы метаданных .nfo, совместимые с Kodi, Jellyfin и Emby, рядом с импортированными медиафайлами; существующий файл .nfo никогда не перезаписывается",
  "settings.plexmatchWriteOnImportLabel": "Создавать .plexmatch при импорте",
  "settings.plexmatchWriteOnImportDescription":
    "Создавать файлы-подсказки Plex для точного сопоставления сериалов без определения по имени",
  "settings.minAvailabilityLabel": "Минимальная доступность",
  "settings.minAvailability.announced": "Анонсировано",
  "settings.minAvailability.in_cinemas": "В кинотеатрах",
  "settings.minAvailability.released": "Выпущено",
  "status.titleRequired": "Необходимо указать название.",
  "status.facetRequired": "Необходимо указать категорию.",
  "status.catalogAddSuccess": "{{name}} добавлено в каталог.",
  "status.catalogAddSuccessAutoSearch":
    "{{name}} добавлено в каталог. Автоматический поиск поставлен в очередь.",
  "status.requestSubmitted": "Запрос для {{name}} отправлен.",
  "status.selectedTvdb":
    "Выбрано совпадение TVDB: {{name}}. Теперь нажмите «Поиск NZBGeek».",
  "status.noReleaseSource":
    "Релиз не содержит источника, который можно добавить в очередь.",
  "status.queueSuccess": "{{name}} добавлено в очередь",
  "status.primaryMovieFileUpdated": "Основной файл фильма обновлён",
  "status.noSource": "Нет источника для добавления {{name}} в очередь",
  "status.noReleaseForTitle": "Для {{name}} релиз не найден",
  "status.queuedLatest": "Последний релиз для {{name}} добавлен в очередь",
  "status.queuedForTitle": "Релизы {{name}} добавлены в очередь.",
  "status.nothingFound": "Результаты не найдены.",
  "status.searchingByQuery": "Поиск по запросу «{{query}}».",
  "status.searchingNzb": "Поиск NZB по запросу «{{query}}»{{category}}.",
  "status.searchingTvdb": "Поиск метаданных TVDB для «{{query}}».",
  "status.foundTvdb": "Найдено результатов TVDB: {{count}}.",
  "status.foundNzb": "Найдено результатов NZBGeek: {{count}}.",
  "status.renamePreviewGenerated":
    "Предварительный просмотр готов: можно переименовать {{renamable}} из {{total}} файлов.",
  "status.renameApplied":
    "Переименование завершено: применено — {{applied}}, пропущено — {{skipped}}, ошибок — {{failed}}.",
  "status.nzbFoundForTitle":
    "Найдено NZB-элементов для {{name}}: {{count}}{{source}}.",
  "status.noNzbFound": "Результаты NZB не найдены.",
  "status.releaseMissingCandidateToken":
    "В выбранном релизе отсутствует подтверждённый токен кандидата.",
  "status.deleteCatalogConfirm":
    "Удалить {{name}} из каталога? Используйте параметр ниже, чтобы также удалить локальные файлы.",
  "status.titleDeleted": "{{name}} удалено из каталога.",
  "status.bulkMonitorSuccess":
    "Включено отслеживание для {{count}} материалов.",
  "status.bulkMonitorPartial":
    "Включено отслеживание для {{count}} материалов; ошибок: {{failed}}.",
  "status.bulkMonitorFailed":
    "Не удалось включить отслеживание для выбранных материалов.",
  "status.bulkUnmonitorSuccess":
    "Отключено отслеживание для {{count}} материалов.",
  "status.bulkUnmonitorPartial":
    "Отключено отслеживание для {{count}} материалов; ошибок: {{failed}}.",
  "status.bulkUnmonitorFailed":
    "Не удалось отключить отслеживание для выбранных материалов.",
  "status.bulkTitleUpdateSuccess": "Обновлено материалов: {{count}}.",
  "status.bulkTitleUpdatePartial":
    "Обновлено материалов: {{count}}; ошибок: {{failed}}.",
  "status.bulkTitleUpdateFailed": "Не удалось обновить выбранные материалы.",
  "status.bulkDeletePreviewFailed":
    "Не удалось подготовить предварительный просмотр удаления для {{failed}} выбранных материалов.",
  "status.bulkTitleDeleteSuccess": "Удалено материалов: {{count}}.",
  "status.bulkTitleDeletePartial":
    "Удалено материалов: {{count}}; ошибок: {{failed}}.",
  "status.bulkTitleDeleteFailed": "Не удалось удалить выбранные материалы.",
  "status.titleMatchUpdated": "Соответствие для {{name}} обновлено.",
  "status.qualityProfileBlocked":
    "Пропущено: заблокировано профилем качества ({{reason}})",
  "status.tvdbRequiredImdb":
    "У выбранного фильма отсутствует подходящий идентификатор IMDb.",
  "status.tvdbNoValidId":
    "У выбранного совпадения TVDB отсутствует корректный идентификатор TVDB.",
  "status.tvdbNeedsTitle":
    "У выбранного совпадения TVDB отсутствует название для поиска.",
  "status.tvdbQueueTip": "Выберите совпадение TVDB, чтобы продолжить.",
  "status.nzbQueueTip":
    "Результаты NZB отсутствуют. Попробуйте выполнить поиск по другому названию.",
  "status.languageChanged": "Язык интерфейса изменён на {{language}}.",
  "status.editingUser": "Редактирование пользователя {{name}}",
  "status.editingIndexer": "Редактирование индексатора {{name}}",
  "status.editingDownloadClient": "Редактирование клиента загрузки {{name}}",
  "status.editingSubtitleProvider":
    "Редактирование провайдера субтитров {{name}}",
  "status.deletingUser": "Удалить пользователя {{name}}?",
  "status.deletingUserListsWarning":
    "Списки, на которые подписан этот пользователь, и история их синхронизации тоже будут удалены.",
  "status.userLoginEnabled": "Вход для пользователя {{name}} включён.",
  "status.userLoginDisabled": "Вход для пользователя {{name}} отключён.",
  "status.deletingIndexer": "Удалить индексатор {{name}}?",
  "status.deletingDownloadClient": "Удалить клиент загрузки {{name}}?",
  "status.deletingSubtitleProvider": "Удалить провайдера субтитров {{name}}?",
  "status.showingActivityStream": "Показать ленту активности",
  "status.hidingActivityStream": "Скрыть ленту активности",
  "status.userRequired": "Необходимо указать имя пользователя и пароль.",
  "status.passwordRequired": "Необходимо указать пароль.",
  "status.failedToCreate": "Не удалось создать элемент.",
  "status.failedToDelete": "Не удалось удалить элемент.",
  "status.failedToLoad": "Не удалось загрузить данные.",
  "status.failedToUpdate": "Не удалось обновить элемент.",
  "status.managePermissionsRequired":
    "Для редактирования разрешений требуется право «Управление разрешениями».",
  "status.indexerCreated": "Индексатор сохранён.",
  "status.indexerUpdated": "Индексатор обновлён.",
  "status.indexerSynced": "Синхронизированы индексаторы, управляемые {{name}}.",
  "status.mismatchRecoveryQueued":
    "В очередь добавлено элементов восстановления несоответствий: {{count}}.",
  "status.titleScanSuccess":
    "Сканирование завершено. Импортировано: {{imported}}, пропущено: {{skipped}}, не сопоставлено: {{unmatched}}.",
  "status.titleMonitoringEnabled": "Мониторинг материала включён.",
  "status.titleMonitoringDisabled": "Мониторинг материала отключён.",
  "status.indexerDeleted": "Индексатор {{name}} удалён.",
  "plugins.failure.invalidConfig":
    "Плагин не может использовать эти настройки. Проверьте их и повторите попытку.",
  "plugins.failure.authFailed":
    "Сервис отклонил данные для входа. Проверьте API-ключ, имя пользователя или пароль.",
  "plugins.failure.rateLimited":
    "Сервис попросил Scryer снизить частоту запросов. Повторите попытку немного позже.",
  "plugins.failure.upstreamUnavailable":
    "Не удалось связаться с сервисом, или он не ответил. Проверьте адрес и повторите попытку.",
  "plugins.failure.unsupported": "Плагин не поддерживает эту операцию.",
  "plugins.failure.temporary":
    "У плагина возникла временная проблема. Повторите попытку немного позже.",
  "plugins.failure.permanent":
    "Плагину не удалось выполнить этот запрос. Повторная попытка не поможет, пока что-то не изменится.",
  "plugins.failure.hostFailure":
    "Плагин неожиданно остановился. Подробности есть в журнале Scryer.",
  "plugins.failure.timedOut": "Плагин слишком долго не отвечал.",
  "plugins.failure.resourceLimit": "Плагину не хватило памяти.",
  "plugins.failure.withDetail": "{{reason}} Подробности: {{detail}}",
  "plugins.failure.invalidUsernamePassword":
    "Сервис отклонил имя пользователя или пароль. Проверьте их и повторите попытку.",
  "plugins.failure.invalidApiKey":
    "Сервис отклонил API-ключ. Проверьте ключ и повторите попытку.",
  "plugins.failure.invalidPasskey":
    "Сервис отклонил passkey. Проверьте passkey и повторите попытку.",
  "plugins.failure.invalidCookie":
    "Сервис отклонил cookie сеанса. Скопируйте свежий cookie из браузера и повторите попытку.",
  "plugins.failure.captchaRequired":
    "Сервис запрашивает CAPTCHA. Войдите один раз на его сайте, затем повторите попытку.",
  "plugins.failure.twoFactorRequired":
    "Для этого входа сервис требует код двухфакторной аутентификации. Используйте вместо этого API-ключ или пароль приложения.",
  "plugins.failure.accountBanned":
    "Сервис заблокировал эту учётную запись. Обратитесь в сервис, чтобы решить проблему.",
  "plugins.failure.accountDisabled":
    "Сервис отключил эту учётную запись. Проверьте учётную запись на сайте сервиса.",
  "plugins.failure.endpointNotFound":
    "Сервис не найден по этому адресу. Проверьте URL и базовый путь.",
  "plugins.failure.tlsError":
    "Не удалось установить защищённое соединение с сервисом. Проверьте адрес и его сертификат.",
  "plugins.failure.invalidResponse":
    "Сервис прислал ответ, который плагин не смог прочитать. Убедитесь, что адрес указывает на нужный сервис.",
  "plugins.failure.quotaExceeded":
    "Учётная запись исчерпала квоту сервиса. Дождитесь её сброса или увеличьте лимит.",
  "plugins.failure.itemNotFound": "Этого элемента в сервисе больше нет.",
  "plugins.failure.downloadRejected": "Клиент загрузки отклонил эту загрузку.",
  "plugins.failure.pathNotWritable":
    "Плагин не смог записать данные в указанную папку. Убедитесь, что она существует и доступна для записи.",
  "plugins.failure.unknown": "Плагин сообщил о проблеме: {{code}}.",
  "status.testingIndexerConnection": "Проверка подключения...",
  "status.indexerConnectionTestPassed":
    "Подключение к индексатору успешно проверено.",
  "status.indexerConnectionTestFailed":
    "Не удалось проверить подключение к индексатору.",
  "status.testingSubtitleProviderConnection":
    "Проверка подключения к провайдеру субтитров...",
  "status.subtitleProviderConnectionTestPassed":
    "Подключение к провайдеру субтитров успешно проверено.",
  "status.subtitleProviderConnectionTestFailed":
    "Не удалось проверить подключение к провайдеру субтитров.",
  "status.ruleCreated": "Правило сохранено.",
  "status.ruleUpdated": "Правило обновлено.",
  "status.ruleDeleted": "Правило {{name}} удалено.",
  "status.ruleToggled": "Правило {{name}}: {{state}}.",
  "status.editingRule": "Редактирование правила {{name}}",
  "status.deletingRule": "Удалить правило {{name}}?",
  "status.mediaServerCreated": "Подключение к медиасерверу сохранено.",
  "status.mediaServerUpdated": "Подключение к медиасерверу обновлено.",
  "status.mediaServerDeleted": "Медиасервер {{name}} удалён.",
  "status.plexServersDiscovered": "Серверы Plex загружены.",
  "status.editingMediaServer": "Редактирование медиасервера {{name}}",
  "status.deletingMediaServer": "Удалить медиасервер {{name}}?",
  "status.testingMediaServerConnection": "Проверка подключения к {{server}}...",
  "status.mediaServerConnectionTestPassed":
    "Подключение к {{server}} успешно проверено.",
  "status.mediaServerConnectionTestFailed":
    "Не удалось проверить подключение к {{server}}.",
  "status.downloadClientCreated": "Клиент загрузки сохранён.",
  "status.downloadClientUpdated": "Клиент загрузки обновлён.",
  "status.downloadClientDeleted": "Клиент загрузки {{name}} удалён.",
  "status.downloadClientDeletedWithMappings":
    "Клиент загрузки {{name}} удалён. {{count}} индексаторов переключены в режим «Автоматически».",
  "status.indexerDownloadClientMappingSaving":
    "Сохранение маршрутизации клиента загрузки...",
  "status.indexerDownloadClientMappingSaved":
    "Маршрутизация клиента загрузки для {{name}} сохранена.",
  "status.downloadClientOrderSaved":
    "Порядок приоритетов клиентов загрузки сохранён.",
  "status.testingDownloadClient": "Проверка подключения к {{client}}...",
  "status.downloadClientConnectionTestPassed":
    "Подключение к {{client}} успешно проверено.",
  "status.downloadClientConnectionTestFailed":
    "Не удалось проверить подключение к {{client}}.",
  "status.testingJellyfinConnection":
    "Проверка подключения к {{connection}}...",
  "status.jellyfinConnectionTestPassed":
    "Подключение к {{connection}} успешно проверено.",
  "status.jellyfinConnectionTestFailed":
    "Не удалось проверить подключение к {{connection}}.",
  "status.autoSearchNoCandidates":
    "Релиз для {{name}} не найден: ни один индексатор не вернул кандидатов. Проверьте категории маршрутизации и состояние индексаторов.",
  "status.autoSearchAllRejected":
    "Релиз для {{name}} не найден: автоматический поиск отклонил все {{count}} кандидатов. {{reasons}}",
  "status.queueFailed": "Операция с очередью завершилась ошибкой.",
  "status.apiError": "Ошибка запроса.",
  "status.permissionDenied": "У вас нет прав для выполнения этого действия.",
  "status.generic": "{{message}}",
  "system.notReadyTemplate": "Система не готова.",
  "system.recentEventsLabel": "Примеры последних событий",
  "user.created": "Пользователь {{name}} создан.",
  "user.permissionsUpdated": "Разрешения для пользователя {{name}} обновлены.",
  "user.passwordUpdated": "Пароль пользователя {{name}} обновлён.",
  "user.mfaReset": "MFA для пользователя {{name}} сброшена.",
  "profile.accountInfo": "Учётная запись",
  "profile.operator": "Оператор",
  "profile.member": "Участник",
  "profile.localAccount": "Локальная",
  "profile.externalAccount": "Внешняя",
  "profile.appearance": "Внешний вид",
  "profile.appearanceDescription":
    "Настройте внешний вид Scryer для своей учётной записи.",
  "profile.highlightColor": "Цвет выделения",
  "profile.highlightColorHelp":
    "Используется для выделения выбранных элементов, активной навигации, значков и основных действий.",
  "profile.highlightColorCustom": "Пользовательский цвет",
  "profile.highlightColorModel": "Цветовая модель",
  "profile.highlightColorApply": "Применить цвет",
  "profile.highlightColorCancel": "Отмена",
  "profile.highlightColorSaved": "Цвет выделения обновлён.",
  "profile.highlightColorSaveFailed": "Не удалось обновить цвет выделения.",
  "color.indigo": "Индиго",
  "color.violet": "Фиолетовый",
  "color.sky": "Небесный",
  "color.teal": "Бирюзовый",
  "color.emerald": "Изумрудный",
  "color.green": "Зелёный",
  "color.amber": "Янтарный",
  "color.cyan": "Циан",
  "color.rose": "Розовый",
  "color.red": "Красный",
  "color.fuchsia": "Фуксия",
  "color.magenta": "Маджента",
  "color.crab": "Крабовый",
  "profile.changePassword": "Изменить пароль",
  "profile.currentPassword": "Текущий пароль",
  "profile.newPassword": "Новый пароль",
  "profile.confirmPassword": "Подтвердите пароль",
  "profile.passwordMismatch": "Пароли не совпадают",
  "profile.passwordUpdated": "Пароль успешно обновлён.",
  "profile.passkeys": "Ключи доступа",
  "profile.passkeysDescription":
    "Добавляйте и удаляйте ключи доступа для более быстрого входа на ваших устройствах.",
  "profile.passkeysEmpty": "Ключи доступа ещё не добавлены.",
  "profile.passkeyAdd": "Добавить ключ доступа",
  "profile.passkeyAdding": "Добавление ключа доступа…",
  "profile.passkeyLabel": "Ключ доступа",
  "profile.passkeyCreatedAt": "Создан",
  "profile.passkeyLastUsedAt": "Последнее использование",
  "profile.passkeyNeverUsed": "Никогда не использовался",
  "profile.passkeyAdded": "Ключ доступа добавлен.",
  "profile.passkeyDeleted": "Ключ доступа удалён.",
  "profile.passkeyDeleteFailed": "Не удалось удалить ключ доступа.",
  "profile.passkeyOperationFailed": "Ошибка запроса ключа доступа.",
  "profile.totp": "Приложение-аутентификатор",
  "profile.totpDescription":
    "Используйте одноразовые коды из 1Password или другого приложения-аутентификатора для дополнительной проверки.",
  "profile.totpStartEnrollment": "Настроить аутентификатор",
  "profile.totpOpenSetupLink": "Открыть ссылку для настройки",
  "profile.totpSecret": "Ключ настройки",
  "profile.totpCode": "Одноразовый код",
  "profile.totpVerifyAndEnable": "Проверить и включить",
  "profile.totpEnabled": "Приложение-аутентификатор включено.",
  "profile.totpDisabled": "Приложение-аутентификатор отключено.",
  "profile.totpEnabledAt": "Включено",
  "profile.totpLastUsedAt": "Последнее использование",
  "profile.totpRecoveryRemaining": "Осталось кодов восстановления",
  "profile.totpRecoveryCodes": "Коды восстановления",
  "profile.totpActionDescription":
    "Введите код аутентификатора для управления этим приложением.",
  "profile.totpDisable": "Отключить аутентификатор",
  "profile.totpRegenerateRecoveryCodes": "Создать новые коды восстановления",
  "profile.totpRecoveryCodesRegenerated": "Коды восстановления обновлены.",
  "profile.totpOperationFailed": "Ошибка запроса аутентификатора.",
  "profile.externalConnections": "Внешние подключения",
  "profile.externalConnectionsPlaceholder":
    "Здесь будут доступны ключи доступа и подключения внешних учётных записей.",
  "profile.linkedAccounts": "Связанные учётные записи",
  "profile.linkedAccountsDescription":
    "Управляйте учётными записями Jellyfin, связанными с этой учётной записью Scryer.",
  "profile.linkedAccountsEmpty": "Внешние учётные записи ещё не связаны.",
  "profile.linkedAccountsLoadFailed":
    "Не удалось загрузить связанные учётные записи.",
  "profile.linkAccountLoadFailed":
    "Не удалось загрузить параметры подключения.",
  "profile.linkedAccountConnection": "Подключение",
  "profile.linkedAccountUnknownConnection": "Неизвестное подключение",
  "profile.linkedAccountStatus": "Статус",
  "profile.linkJellyfinAccount": "Подключить Jellyfin",
  "profile.linkPlexAccount": "Подключить Plex",
  "profile.linkAccountConnection": "Подключение",
  "profile.linkAccountUsername": "Имя пользователя",
  "profile.linkAccountPassword": "Пароль",
  "profile.linkAccountPasswordlessHint":
    "Оставьте поле пустым, если для этой учётной записи Jellyfin пароль не используется. Учётные записи без пароля можно привязать, но их нельзя использовать для входа в Scryer.",
  "profile.linkAccountSubmit": "Подключить учётную запись",
  "profile.linkAccountCancel": "Отмена",
  "profile.linkAccountNoConnections": "Нет доступных подключений.",
  "profile.linkAccountLinked": "Внешняя учётная запись подключена.",
  "profile.linkAccountFailed": "Не удалось подключить внешнюю учётную запись.",
  "profile.signInWithPlexToLink": "Войти через Plex",
  "profile.unlinkAccount": "Отвязать",
  "profile.linkedAccountUnlinked": "Связанная учётная запись удалена.",
  "profile.linkedAccountUnlinkFailed": "Не удалось отвязать учётную запись.",
  "auth.loginTitle": "Вход",
  "auth.signIn": "Войти",
  "auth.signingIn": "Выполняется вход…",
  "auth.usernamePlaceholder": "Имя пользователя",
  "auth.username": "Имя пользователя",
  "auth.passwordPlaceholder": "Пароль",
  "auth.password": "Пароль",
  "auth.totpCode": "Одноразовый код",
  "auth.loginButton": "Войти",
  "auth.loggingIn": "Выполняется вход…",
  "auth.loginFailed": "Неверное имя пользователя или пароль.",
  "auth.invalidCredentials": "Неверное имя пользователя или пароль.",
  "auth.signInFailedGeneric":
    "Не удалось выполнить вход. Проверьте введённые данные и попробуйте снова.",
  "auth.signInRateLimited":
    "Слишком много попыток входа. Подождите немного и повторите попытку.",
  "auth.signInWithScryerPassword": "Войти с помощью пароля Scryer",
  "auth.signInWithPasskey": "Войти с помощью ключа доступа",
  "auth.signInWithJellyfin": "Войти через Jellyfin",
  "auth.jellyfinFailed": "Не удалось выполнить вход через Jellyfin.",
  "auth.signInWithPlex": "Войти через Plex",
  "auth.plexFailed": "Не удалось выполнить вход через Plex.",
  "auth.plexPinFlowPending": "Завершите вход в открывшемся окне Plex.",
  "auth.passkeySigningIn": "Выполняется вход с помощью ключа доступа…",
  "auth.passkeyUnsupported": "Этот браузер не поддерживает ключи доступа.",
  "auth.passkeyCancelled": "Запрос ключа доступа был отменён.",
  "auth.passkeyFailed": "Ошибка при использовании ключа доступа.",
  "auth.totpCodeRequired":
    "Введите одноразовый код Scryer, чтобы завершить вход.",
  "auth.mfaSetupTitle": "Настройка MFA в Scryer",
  "auth.mfaSetupDescription":
    "Вход выполнен успешно. Настройте одноразовые коды Scryer сейчас, чтобы в будущем можно было подтверждать вход.",
  "auth.mfaSetupStartFailed": "Не удалось начать настройку MFA.",
  "auth.mfaSetupCompleteFailed": "Не удалось завершить настройку MFA.",
  "auth.mfaSetupCancel": "Вернуться к входу",
  "auth.mfaSetupRestart": "Начать настройку MFA заново",
  "auth.mfaRecoveryCodesDescription":
    "Сохраните эти коды восстановления в безопасном месте. Каждый код можно использовать один раз, если у вас не будет доступа к приложению-аутентификатору.",
  "auth.continue": "Продолжить",
  "auth.logoutButton": "Выйти",
  "auth.sessionExpired":
    "Срок действия сеанса истёк. Пожалуйста, войдите снова.",
  "nav.wanted": "Разыскиваемое",
  "nav.requests": "Запросы",
  "requests.empty": "Для этого фильтра нет ожидающих запросов.",
  "requests.emptyMine": "Для этого фильтра нет запросов.",
  "requests.mode.admin": "Очередь",
  "requests.mode.mine": "Мои",
  "requests.approve": "Одобрить",
  "requests.approveTitle": "Одобрить запрос",
  "requests.approvedQualityProfile": "Одобренный профиль качества",
  "requests.approvedMonitorType": "Одобренный режим отслеживания",
  "requests.dismiss": "Отклонить",
  "requests.reopen": "Открыть снова",
  "requests.requesterFilter.label": "Автор запроса",
  "requests.requesterFilter.all": "Все авторы запросов",
  "status.requestReopened": "Запрос на {{name}} снова открыт.",
  "requests.modify": "Изменить",
  "requests.modifyTitle": "Изменить запрос",
  "requests.saveChanges": "Сохранить изменения",
  "requests.cancelRequest": "Отменить запрос",
  "requests.requesters": "Пользователи",
  "requests.requestedQualityProfile": "Запрошенный профиль качества",
  "requests.requestedMonitorType": "Запрошенный режим отслеживания",
  "requests.libraryDefaultProfile": "Профиль библиотеки по умолчанию",
  "requests.updated": "Обновлено",
  "requests.status.pending": "Ожидает",
  "requests.status.approved": "Одобрено",
  "requests.status.rejected": "Отклонено",
  "requests.status.canceled": "Отменено",
  "status.requestApproved": "{{name}} одобрено.",
  "status.requestApprovedSearchFailed":
    "{{name}} одобрено, но поиск не был добавлен в очередь: {{error}}",
  "status.requestDismissed": "Запрос для {{name}} отклонён.",
  "status.requestUpdated": "Запрос для {{name}} обновлён.",
  "status.requestCanceled": "Запрос для {{name}} отменён.",
  "wanted.title": "Разыскиваемые элементы",
  "wanted.refreshing": "Обновление…",
  "wanted.filterTitle": "Фильтр по названию",
  "wanted.filterTitlePlaceholder": "Фильтр по названиям…",
  "wanted.totalCount": "{{count}} элементов",
  "wanted.colTitle": "Название",
  "wanted.colType": "Тип",
  "wanted.colStatus": "Статус",
  "wanted.colConvergence": "Состояние",
  "wanted.colLastSearch": "Последний поиск",
  "wanted.colIndexers": "Индексаторы",
  "wanted.convergence.searching": "Поиск {{covered}}/{{routed}}",
  "wanted.convergence.converged": "Отслеживание через RSS",
  "wanted.convergence.convergedHint":
    "Поиск выполнен по всем индексаторам — новые релизы будут поступать через RSS. Запустите поиск повторно, чтобы проверить ещё раз.",
  "wanted.convergence.deferred": "Ожидание квоты индексатора",
  "wanted.convergence.deferredHint":
    "Все оставшиеся индексаторы находятся в режиме ожидания или исчерпали квоту. Поиск автоматически возобновится позже.",
  "wanted.convergence.queuedHot": "В очереди · недавно",
  "wanted.convergence.queuedCold": "В очереди · длительный поиск",
  "wanted.searchJobStarted": "Поиск запущен.",
  "wanted.searchJobComplete":
    "Поиск завершён: проверено — {{processed}}, загружено — {{grabbed}}, ошибок — {{failed}}.",
  "wanted.searchJobCancelled":
    "Поиск отменён после обработки {{processed}} элементов; загружено — {{grabbed}}.",
  "wanted.colPhase": "Этап",
  "wanted.colLatestDecision": "Последнее решение",
  "wanted.colScore": "Оценка",
  "wanted.searchNow": "Искать сейчас",
  "wanted.pause": "Приостановить",
  "wanted.resume": "Продолжить",
  "wanted.actionRecoverMismatch": "Исправить несоответствие",
  "wanted.status.wanted": "Требуется",
  "wanted.status.grabbed": "Загружено",
  "wanted.status.completed": "Завершено",
  "wanted.status.paused": "Приостановлено",
  "wanted.type.movie": "Фильм",
  "wanted.type.episode": "Эпизод",
  "wanted.type.seriesMovie": "Фильм сериала",
  "wanted.decision.eligible": "Подходит",
  "wanted.decision.titleMismatch": "Несоответствие названия",
  "wanted.decision.episodeMismatch": "Несоответствие эпизода",
  "wanted.decision.categoryMismatch": "Несоответствие категории",
  "wanted.decision.ambiguousIdentity": "Неоднозначное совпадение",
  "wanted.decision.qualityBlocked": "Заблокировано профилем качества",
  "wanted.decision.upgradeRejected": "Обновление отклонено",
  "wanted.decision.pendingDelay": "Отложено",
  "wanted.decision.alreadyActive": "Уже активно",
  "wanted.decision.acceptInitial": "Первичное принятие",
  "wanted.decision.acceptUpgrade": "Обновление принято",
  "wanted.decision.rejectInsufficientDelta": "Недостаточная разница",
  "wanted.decision.rejectCooldown": "Период ожидания",
  "wanted.decision.rejectNotAllowed": "Не разрешено",
  "wanted.loadingDecisions": "Загрузка решений по релизам…",
  "wanted.noDecisions": "Решения по релизам ещё не зарегистрированы.",
  "wanted.noItems": "Разыскиваемые элементы не найдены.",
  "wanted.context.movie": "Разыскиваемый фильм",
  "wanted.context.episode": "Разыскиваемый эпизод",
  "wanted.context.seasonEpisode":
    "Разыскиваемый эпизод сезона {{seasonNumber}}",
  "wanted.context.seriesMovie": "Разыскиваемый фильм сериала",
  "wanted.prev": "Предыдущий",
  "wanted.next": "Следующий",
  "wanted.timeNow": "сейчас",
  "wanted.timeHoursMinutes": "{{hours}} ч {{minutes}} мин",
  "wanted.timeMinutes": "{{minutes}} мин",
  "wanted.decRelease": "Релиз",
  "wanted.decDecision": "Решение",
  "wanted.decScore": "Оценка",
  "wanted.decDelta": "Разница",
  "wanted.decSize": "Размер",
  "wanted.decDate": "Дата",
  "wanted.scoreBreakdown": "Подробности оценки",
  "wanted.scoreCode": "Код",
  "wanted.tabWanted": "Разыскиваемые элементы",
  "wanted.tabCutoff": "Порог качества не достигнут",
  "wanted.tabCalendar": "Календарь",
  "cutoff.title": "Обновления",
  "cutoff.searchAll": "Искать всё",
  "cutoff.searching": "Поиск…",
  "cutoff.searchProgress": "Поиск {{current}} из {{total}}…",
  "cutoff.colTitle": "Название",
  "cutoff.colTitleEpisode": "Название / Эпизод",
  "cutoff.colFacet": "Тип",
  "cutoff.colCurrentQuality": "Текущее качество",
  "cutoff.colTargetQuality": "Целевое качество",
  "cutoff.noItems":
    "Все отслеживаемые фильмы и эпизоды соответствуют пороговому уровню профиля качества.",
  "cutoff.totalCount": "{{count}} элементов ниже порогового уровня",
  "cutoff.filterFacet": "Тип",
  "cutoff.allFacets": "Все типы",
  "cutoff.searchTriggered": "Поиск обновлений для {{name}} запущен",
  "narrative.movie": "Фильм",
  "narrative.canonMovieInstallment":
    "Каноническая часть фильма. Расположена в соответствии с порядком просмотра сюжета.",
  "settings.notifications": "Уведомления",
  "settings.notificationsSection":
    "Управление каналами уведомлений и подписками",
  "settings.notificationChannels": "Каналы",
  "settings.notificationSubscriptions": "Подписки",
  "settings.notificationChannelCreate": "Добавить канал",
  "settings.notificationChannelCreateNew": "Создать новый канал",
  "settings.notificationChannelUpdate": "Обновить канал",
  "settings.notificationSubscriptionCreate": "Добавить подписку",
  "settings.notificationSubscriptionCreateNew": "Создать новую подписку",
  "settings.notificationSubscriptionUpdate": "Обновить подписку",
  "settings.notificationChannelConfirmDiscardTitle":
    "Отменить текущие изменения канала?",
  "settings.notificationChannelConfirmDiscardDescription":
    "Все внесённые изменения канала будут потеряны.",
  "settings.notificationSubscriptionConfirmDiscardTitle":
    "Отменить текущие изменения подписки?",
  "settings.notificationSubscriptionConfirmDiscardDescription":
    "Все внесённые изменения подписки будут потеряны.",
  "settings.notificationNoChannels": "Каналы уведомлений не настроены.",
  "settings.notificationNoSubscriptions":
    "Подписки на уведомления не настроены.",
  "settings.notificationEventType": "Тип события",
  "settings.notificationScope": "Область действия",
  "settings.notificationScope.global": "Глобальная",
  "settings.notificationScope.facet": "Категория",
  "settings.notificationScope.title": "Название",
  "settings.notificationScopeId": "Идентификатор области",
  "settings.notificationScopeIdPlaceholderFacet": "movie, series, anime",
  "settings.notificationScopeIdPlaceholderTitle": "Идентификатор названия",
  "settings.notificationChannel": "Канал",
  "settings.notificationTest": "Проверить",
  "settings.notificationTesting": "Проверка…",
  "settings.notificationTestSuccess":
    "Тестовое уведомление успешно отправлено.",
  "settings.notificationTestFailed":
    "Не удалось отправить тестовое уведомление.",
  "settings.notificationProviderType": "Тип провайдера",
  "settings.notificationNoProviders":
    "Плагины уведомлений не установлены. Сначала установите плагин уведомлений на странице «Плагины».",
  "settings.notificationJellyfinChannelHint":
    "Сопоставления путей должны связывать папки Scryer с путями к папкам, доступным Jellyfin. Оставьте их пустыми, если оба сервиса используют одинаковые пути.",
  "settings.notificationJellyfinSubscriptionHint":
    "Завершение импорта — стандартный триггер для запуска сканирования Jellyfin после импорта. Здесь доступны только события Jellyfin, способные обновить библиотеку.",
  "settings.notificationEvent.grab": "Релиз получен",
  "settings.notificationEvent.download": "Ошибка загрузки",
  "settings.notificationEvent.upgrade": "Обновление импортировано",
  "settings.notificationEvent.importComplete": "Импорт завершён",
  "settings.notificationEvent.importRejected": "Импорт заблокирован",
  "settings.notificationEvent.rename": "Файл переименован",
  "settings.notificationEvent.titleAdded": "Название добавлено",
  "settings.notificationEvent.titleDeleted": "Название удалено",
  "settings.notificationEvent.titleMoved": "Название перемещено",
  "settings.notificationEvent.fileDeleted": "Файл удалён",
  "settings.notificationEvent.fileRestored": "Файл восстановлен",
  "settings.notificationEvent.fileDeletedForUpgrade":
    "Файл удалён для обновления",
  "settings.notificationEvent.postProcessingCompleted":
    "Постобработка завершена",
  "settings.notificationEvent.subtitleDownloaded": "Субтитры загружены",
  "settings.notificationEvent.subtitleSearchFailed":
    "Не удалось найти субтитры",
  "settings.notificationEvent.mediaRequestSubmitted":
    "Запрос на медиаконтент отправлен",
  "settings.notificationEvent.mediaRequestApproved":
    "Запрос на медиаконтент одобрен",
  "settings.notificationEvent.mediaRequestRejected":
    "Запрос на медиаконтент отклонён",
  "settings.notificationEvent.mediaRequestCanceled":
    "Запрос на медиаконтент отменён",
  "settings.notificationEvent.healthIssue": "Обнаружена проблема",
  "settings.notificationEvent.healthRestored":
    "Работоспособность восстановлена",
  "settings.notificationEvent.applicationUpdate": "Обновление приложения",
  "settings.notificationEvent.manualInteractionRequired":
    "Требуется вмешательство пользователя",
  "settings.notificationEvent.test": "Тестовое уведомление",
  "status.notificationChannelCreated": "Канал уведомлений сохранён.",
  "status.notificationChannelUpdated": "Канал уведомлений обновлён.",
  "status.notificationChannelDeleted": "Канал уведомлений {{name}} удалён.",
  "status.notificationSubscriptionCreated":
    "Подписка на уведомления сохранена.",
  "status.notificationSubscriptionUpdated":
    "Подписка на уведомления обновлена.",
  "status.notificationSubscriptionDeleted": "Подписка на уведомления удалена.",
  "status.editingNotificationChannel": "Редактирование канала {{name}}",
  "status.deletingNotificationChannel": "Удалить канал уведомлений {{name}}?",
  "status.editingNotificationSubscription": "Редактирование подписки",
  "status.deletingNotificationSubscription":
    "Удалить эту подписку на уведомления?",
  "wanted.tabPending": "Ожидающие",
  "pending.title": "Ожидающие релизы",
  "pending.noItems": "Нет ожидающих релизов.",
  "pending.colRelease": "Релиз",
  "pending.colScore": "Оценка",
  "pending.colSize": "Размер",
  "pending.colIndexer": "Индексатор",
  "pending.colAddedAt": "Добавлен",
  "pending.colDelayUntil": "Доступен",
  "pending.colReason": "Причина",
  "pending.colRole": "Роль",
  "pending.role.primary": "Основной",
  "pending.role.fallback": "Резервный",
  "pending.status.needsReview": "Требует проверки",
  "pending.phase.needsReview": "Проверка",
  "pending.forceGrab": "Загрузить сейчас",
  "pending.dismiss": "Отклонить",
  "pending.grabbed": "Релиз загружен.",
  "pending.grabRejected":
    "Загрузка отклонена — релиз заблокирован или недоступен.",
  "pending.dismissed": "Релиз отклонён.",
  "pending.timeRemaining": "Осталось: {{time}}",
  "system.rssSync": "Синхронизация RSS",
  "system.rssSyncTrigger": "Запустить синхронизацию RSS",
  "system.rssSyncing": "Синхронизация…",
  "system.rssSyncComplete":
    "Синхронизация RSS: получено — {{fetched}}, совпадений — {{matched}}, загружено — {{grabbed}}, отложено — {{held}}.",
  "setup.welcomeTitle": "Добро пожаловать в Scryer",
  "setup.welcomeDescription":
    "Давайте настроим основные параметры, чтобы начать работу.",
  "setup.freshSetup": "Настроить с нуля",
  "setup.freshSetupDescription":
    "Настройте пути к медиафайлам, установите провайдеров, например qBittorrent, и поэтапно добавьте клиентов и индексаторы.",
  "setup.importSetup": "Импорт из Sonarr / Radarr",
  "setup.importSetupDescription":
    "Перенесите существующую конфигурацию в Scryer.",
  "setup.restoreSetup": "Восстановить из резервной копии",
  "setup.restoreSetupDescription":
    "Загрузите резервную копию Scryer и восстановите этот новый экземпляр.",
  "setup.stepPersona": "Профиль",
  "setup.stepMediaPaths": "Пути к медиафайлам",
  "setup.stepPlugins": "Плагины",
  "setup.stepDownloadClient": "Клиент загрузки",
  "setup.stepIndexer": "Индексатор",
  "setup.stepSummary": "Сводка",
  "setup.stepConnect": "Подключение",
  "setup.stepReview": "Проверка",
  "setup.stepRestore": "Восстановление",
  "setup.restoreTitle": "Восстановление из резервной копии",
  "setup.restoreDescription":
    "Загрузите резервную копию Scryer, просмотрите её содержимое и восстановите данные в этот новый экземпляр.",
  "setup.restoreSelectBundle": "Файл резервной копии",
  "setup.restoreDropTargetTitle": "Перетащите файл резервной копии сюда",
  "setup.restoreDropTargetDescription":
    "Выберите файл резервной копии на устройстве или перетащите его сюда, чтобы просмотреть содержимое перед восстановлением.",
  "setup.restoreDropTargetSelected": "Файл выбран и готов к проверке.",
  "setup.restoreDropTargetEncryptedSelected":
    "Выбран зашифрованный архив. Введите пароль, указанный при создании резервной копии.",
  "setup.restoreDropTargetFormats":
    "Поддерживаются только зашифрованные резервные копии .enc.",
  "setup.restoreSelectFile": "Выбрать файл",
  "setup.restoreClearFile": "Очистить выбор",
  "setup.restoreNoFile": "Выберите файл резервной копии, чтобы продолжить.",
  "setup.restorePasswordRequired":
    "Введите пароль от архива, чтобы продолжить.",
  "setup.restoreInspect": "Проверить архив",
  "setup.restoreApply": "Восстановить",
  "setup.restoreChooseAnother": "Выбрать другой архив",
  "setup.restoreSummaryTitle": "Содержимое резервной копии",
  "setup.restoreConfirmDescription":
    "Будет заменена база данных нового экземпляра, восстановлены секретные данные и выполнен перезапуск Scryer.",
  "setup.restoreCreatedAt": "Создан",
  "setup.restoreSourceVersion": "Версия источника",
  "setup.restoreSourceEngine": "Движок источника",
  "setup.restoreMigrationKey": "Ключ миграции",
  "setup.restoreTotalRows": "Всего записей",
  "setup.personaTitle": "Предпочтения качества",
  "setup.personaDescription":
    "Выберите целевое качество и профиль оценки для каждого типа контента.",
  "setup.facetMovies": "Фильмы",
  "setup.facetSeries": "Сериалы",
  "setup.facetAnime": "Аниме",
  "setup.qualityTarget": "Качество",
  "setup.scoringFocus": "Профиль",
  "setup.personaBalancedDesc":
    "Оптимальные настройки для большинства пользователей. Хорошее качество без приоритета ремуксов.",
  "setup.personaAudiophileDesc":
    "Максимальное качество. Приоритет отдаётся BluRay, ремуксам, аудио без потерь, HDR и файлам большого размера.",
  "setup.personaEfficientDesc":
    "Лучшее качество при минимальном размере. Предпочтение отдаётся x265, WEB-DL и компактным кодировкам.",
  "setup.personaCompatibleDesc":
    "Максимальная совместимость. Предпочтение H.264 перед H.265, отказ от Dolby Vision и поддержка большинства устройств.",
  "setup.pluginsTitle": "Установите провайдеров, которых хотите использовать",
  "setup.pluginsDescription":
    "Некоторые сервисы, например qBittorrent, необходимо установить как плагины, прежде чем их можно будет настроить в Scryer.",
  "setup.pluginsBuiltInTitle": "Уже включено",
  "setup.pluginsBuiltInDescription":
    "NZBGet и SABnzbd уже доступны и не требуют дополнительной установки.",
  "setup.pluginsAvailableHeading": "Официальные плагины",
  "setup.pluginsAvailableHint":
    "Установите официальные плагины, которые планируете использовать на этом сервере. Позже их можно изменить в настройках.",
  "setup.pluginsInstalledHeading": "Установлено на этом сервере",
  "setup.pluginsAvailable": "Доступные плагины",
  "setup.pluginsNoneFound":
    "Плагины не найдены. Обновите реестр или продолжайте использовать встроенные провайдеры.",
  "setup.pluginsNoneAvailable": "Все плагины из реестра уже установлены.",
  "setup.mediaPathsTitle": "Где хранятся ваши медиафайлы?",
  "setup.mediaPathsDescription":
    "Укажите корневые папки, в которых Scryer будет организовывать ваши фильмы и сериалы.",
  "setup.moviesPath": "Путь к фильмам",
  "setup.seriesPath": "Путь к сериалам",
  "setup.animePath": "Путь к аниме",
  "setup.optional": "(необязательно)",
  "setup.browse": "Обзор",
  "setup.mediaPathNotReachable": "Путь пока недоступен",
  "setup.mediaPathsNotReachableWarning":
    "Некоторые пути к медиафайлам пока недоступны. Они были сохранены, и их можно изменить позже в настройках.",
  "setup.mediaPathsVerificationUnavailable":
    "Не удалось проверить указанные пути. Вы можете продолжить настройку и изменить их позже в настройках.",
  "setup.downloadClientTitle": "Добавьте клиент загрузки",
  "setup.downloadClientDescription":
    "Scryer отправляет загрузки в выбранный вами клиент. Плагины, установленные на предыдущем этапе, появятся здесь.",
  "setup.indexerTitle": "Добавьте индексатор",
  "setup.indexerDescription":
    "Индексаторы предоставляют базу данных релизов, в которой выполняет поиск Scryer.",
  "setup.summaryTitle": "Всё готово",
  "setup.summaryDescription":
    "Ниже приведены выбранные вами параметры. Вы сможете изменить их позже в настройках.",
  "setup.summaryPersona": "Профиль",
  "setup.summaryMediaPaths": "Пути к медиафайлам",
  "setup.summaryDownloadClient": "Клиент загрузки",
  "setup.summaryIndexer": "Индексатор",
  "setup.summaryConfigured": "Настроено",
  "setup.next": "Далее",
  "setup.back": "Назад",
  "setup.finish": "Завершить настройку",
  "setup.importOnly": "Только импорт",
  "setup.importAndScan": "Импортировать и просканировать",
  "setup.connectionSuccess": "Подключение успешно установлено",
  "setup.connectionFailed": "Не удалось установить подключение",
  "setup.saved": "Сохранено",
  "setup.required": "Это поле обязательно",
  "setup.skip": "Пропустить",
  "setup.skipSetup": "Полностью пропустить настройку",
  "setup.skipping": "Пропуск…",
  "setup.connectTitle": "Подключение к Sonarr, Radarr и Prowlarr",
  "setup.connectDescription":
    "Введите URL-адрес и API-ключ для Sonarr, Radarr и/или Prowlarr. Необходимо настроить хотя бы один сервис.",
  "setup.sonarrUrl": "URL-адрес Sonarr",
  "setup.sonarrApiKey": "API-ключ Sonarr",
  "setup.radarrUrl": "URL-адрес Radarr",
  "setup.radarrApiKey": "API-ключ Radarr",
  "setup.prowlarrUrl": "URL-адрес Prowlarr",
  "setup.prowlarrApiKey": "API-ключ Prowlarr",
  "setup.findApiKey": "Найти API-ключ",
  "setup.connectAndScan": "Подключить и просканировать",
  "setup.connectError": "Ошибка подключения",
  "setup.atLeastOneRequired":
    "Укажите хотя бы один сервис: Sonarr, Radarr или Prowlarr.",
  "setup.urlPlaceholder": "http://localhost:8989",
  "setup.reviewTitle": "Проверка и импорт",
  "setup.reviewDescription":
    "Выберите конфигурации, которые нужно перенести в Scryer.",
  "setup.connected": "подключено",
  "setup.mediaPathsSection": "Пути к медиафайлам",
  "setup.moviesPathFrom": "Путь к фильмам (из Radarr)",
  "setup.seriesPathFrom": "Путь к сериалам (из Sonarr)",
  "setup.downloadClientsSection": "Клиенты загрузки",
  "setup.indexersSection": "Индексаторы",
  "setup.notSupported": "Не поддерживается",
  "setup.fromSonarr": "Sonarr",
  "setup.fromRadarr": "Radarr",
  "setup.fromProwlarr": "Prowlarr",
  "setup.importSelected": "Импортировать выбранное",
  "setup.importing": "Импорт…",
  "setup.noneSelected": "Выберите хотя бы один элемент для импорта.",
  "setup.noItemsFound": "Элементы не найдены.",
  "setup.animePathFrom": "Путь к аниме (из Sonarr, необязательно)",
  "setup.none": "Нет",
  "setup.customFormatsHint":
    "Используете пользовательские форматы? Многие из этих возможностей доступны в разделе «Профиль». Дополнительные настройки можно выполнить после первоначальной настройки.",
  "setup.summaryImportedClients": "Импортировано клиентов загрузки",
  "setup.summaryImportedIndexers": "Импортировано индексаторов",
  "setup.monitorWarmupTitle": "Подготовка импорта состояния отслеживания",
  "setup.monitorWarmupDescription":
    "Scryer в фоновом режиме получает состояние отслеживания, чтобы ускорить заключительный этап импорта.",
  "setup.monitorWarmupQueued": "Ожидание данных о ходе выполнения…",
  "setup.monitorWarmupLoadingIndexers": "Обнаружение индексаторов",
  "setup.monitorWarmupLoadingMovies": "Загрузка фильмов",
  "setup.monitorWarmupLoadingSeries": "Загрузка сериалов",
  "setup.monitorWarmupLoadingEpisodes": "Загрузка эпизодов",
  "setup.monitorWarmupBuildingSnapshot": "Создание снимка состояния",
  "setup.monitorWarmupReady": "Готово",
  "setup.monitorWarmupFailed":
    "Во время фоновой подготовки произошла ошибка. Scryer повторит попытку при запуске импорта.",
  "setup.monitorWarmupCanceled":
    "Фоновая подготовка была отменена. При необходимости Scryer запустит её снова.",
  "setup.importFinalizeFailed":
    "Scryer не удалось завершить подготовку состояния отслеживания для этого импорта.",
  "setup.apiKeyMasked":
    "Sonarr/Radarr скрывает этот API-ключ. Введите его вручную, чтобы продолжить импорт:",
  "setup.apiKeyPlaceholder": "Вставьте API-ключ…",
  "setup.apiKeyHelpLink": "Найдите его в SABnzbd → Config → General",
  "setup.prowlarrManagedChildren": "Управляемые индексаторы",
  "setup.prowlarrManagedChildrenReadOnly":
    "Управляемые индексаторы отображаются только для проверки и создаются при синхронизации родительской конфигурации Prowlarr.",
  "setup.prowlarrApiKeyMasked":
    "Sonarr/Radarr скрывает API-ключ Prowlarr или обнаруженные ключи конфликтуют. Введите API-ключ Prowlarr из раздела Settings → General:",
  "setup.prowlarrApiKeyRequired":
    "Введите API-ключ Prowlarr для {{name}} перед импортом.",
  "setup.prowlarrApiKeyHelpLink": "Найдите его в Prowlarr → Settings → General",
  "setup.stepLibraries": "Библиотеки",
  "setup.stepQuality": "Качество",
  "setup.stepSources": "Источники",
  "setup.continue": "Продолжить",
  "setup.finishImport": "Завершить импорт",
  "setup.connectMultiTitle": "Подключение к Sonarr, Radarr и Prowlarr",
  "setup.connectMultiDescription":
    "Добавьте URL-адрес и API-ключ для каждого экземпляра, из которого Scryer должен импортировать данные. Можно подключить несколько экземпляров каждого сервиса, но требуется как минимум один.",
  "setup.connectReadOnlyNote":
    "Scryer только считывает существующую конфигурацию — никакие изменения не будут записаны в Sonarr, Radarr или Prowlarr.",
  "setup.librariesTitle": "Библиотеки и сопоставление",
  "setup.librariesDescription":
    "Создайте отдельную библиотеку для каждого способа организации контента, а затем сопоставьте с ней все корневые каталоги источников.",
  "setup.qualityPersonaTitle": "Качество и профиль",
  "setup.qualityPersonaDescription":
    "Выберите профиль качества и профиль оценки для каждой сопоставленной библиотеки.",
  "setup.sourcesTitle": "Клиенты и индексаторы",
  "setup.sourcesDescription":
    "Выберите, какие клиенты загрузки и индексаторы необходимо импортировать.",
  "setup.importReadyTitle": "Всё готово к импорту",
  "setup.importReadyDescription":
    "Проверьте настройки перед тем, как Scryer выполнит импорт.",
  "setup.productSonarr": "Sonarr",
  "setup.productRadarr": "Radarr",
  "setup.productProwlarr": "Prowlarr",
  "setup.addInstance": "Добавить {{product}}",
  "setup.instanceName": "Имя экземпляра",
  "setup.unnamedInstance": "Безымянный экземпляр",
  "setup.removeInstance": "Удалить экземпляр",
  "setup.testing": "Проверка…",
  "setup.couldntConnect": "Не удалось подключиться",
  "setup.enterUrlAndKey": "Введите URL-адрес и API-ключ для проверки",
  "setup.apiKeyHelpHint": "Настройки → Основные",
  "setup.secretDraftOtherUserWarning":
    "У другого пользователя сохранён черновик импорта. Ввод собственных ключей заменит сохранённые учётные данные.",
  "setup.instanceCountOne": "1 экземпляр",
  "setup.instanceCountMany": "{{count}} экземпляров",
  "setup.sourceRoots": "Корневые каталоги источников",
  "setup.addSourceRoot": "Добавить корневой каталог",
  "setup.chooseFolder": "Выбрать папку…",
  "setup.unmappedCountOne": "1 несопоставленный каталог",
  "setup.unmappedCountMany": "{{count}} несопоставленных каталогов",
  "setup.allRootsMapped":
    "Все корневые каталоги сопоставлены. Перетащите каталог обратно сюда, чтобы отменить сопоставление.",
  "setup.dragRootIntoLibrary":
    "Перетащите каждый корневой каталог в соответствующую библиотеку",
  "setup.dropRootsHere": "Перетащите каталоги сюда",
  "setup.removeSourceRoot": "Удалить корневой каталог",
  "setup.manualRootPathAria": "Путь к корневому каталогу, указанный вручную",
  "setup.renameLibraryAria": "Переименовать библиотеку {{name}}",
  "setup.previewLoading":
    "Загрузка корневых каталогов из подключённых экземпляров…",
  "setup.previewError": "Не удалось загрузить корневые каталоги.",
  "setup.retry": "Повторить",
  "setup.noCompatibleLibrary": "Подходящая библиотека пока не создана.",
  "setup.createFacetLibrary": "Создать библиотеку {{facet}}",
  "setup.addLibrary": "Добавить библиотеку",
  "setup.newLibraryFor": "Новая библиотека для…",
  "setup.cancel": "Отмена",
  "setup.renameLibrary": "Переименовать",
  "setup.renameInstance": "Переименовать",
  "setup.deleteLibrary": "Удалить библиотеку",
  "setup.remap": "Переназначить",
  "setup.assignToLibrary": "Назначить библиотеке",
  "setup.moveToAnotherLibrary": "Переместить в другую библиотеку",
  "setup.assignSheetTitle": "Назначить библиотеке",
  "setup.moveToUnassigned": "Переместить в неназначенные",
  "setup.noPathSet": "(путь не указан)",
  "setup.remapTitle": "Переназначение пути",
  "setup.remapExplain":
    "Scryer сохраняет путь, сообщаемый {{instance}}, в качестве исходного, чтобы корректно восстанавливать состояние отслеживания. Если на сервере Scryer эти файлы находятся по другому пути, укажите его ниже.",
  "setup.remapAsReported": "Как указано источником",
  "setup.remapScryerHostPath": "Путь на сервере Scryer",
  "setup.remapResetToSource": "Сбросить к исходному пути",
  "setup.remapSave": "Сохранить",
  "setup.mappedPathValidationChecking":
    "Проверка доступа Scryer ко всем сопоставленным путям источников…",
  "setup.invalidMappedPathTitle":
    "Scryer не может получить доступ к этому сопоставленному пути",
  "setup.invalidMappedPathsTitle":
    "Scryer не может получить доступ к {{count}} сопоставленным путям",
  "setup.invalidMappedPathsHelp":
    "Эти каталоги назначены, но их пути недоступны для Scryer. Перед продолжением переназначьте каждый каталог на соответствующую папку на сервере Scryer.",
  "setup.remapInvalidRootAria": "Переназначить {{name}} из {{path}}",
  "setup.noPath": "(путь отсутствует)",
  "setup.library": "Библиотека",
  "setup.qualityProfile": "Профиль качества",
  "setup.persona": "Персона",
  "setup.qualityNoLibraries":
    "Сопоставьте хотя бы один корневой каталог с библиотекой, чтобы настроить качество.",
  "setup.persona.balanced.name": "Сбалансированный",
  "setup.persona.balanced.desc": "Хорошее качество при разумном размере файлов",
  "setup.persona.audiophile.name": "Аудиофил",
  "setup.persona.audiophile.desc":
    "Максимальное качество и более крупные файлы",
  "setup.persona.efficient.name": "Эффективный",
  "setup.persona.efficient.desc":
    "Компактные кодировки и меньший размер файлов",
  "setup.persona.compatible.name": "Совместимый",
  "setup.persona.compatible.desc":
    "Максимальная совместимость и поддержка прямого воспроизведения",
  "setup.downloadClients": "Клиенты загрузки",
  "setup.indexers": "Индексаторы",
  "setup.merged": "ОБЪЕДИНЕНО",
  "setup.imported": "Импортировано",
  "setup.from": "ИЗ",
  "setup.passwordRequired": "Требуется пароль.",
  "setup.indexerApiKeyHelpLink": "Найти API-ключ индексатора",
  "setup.provenanceSource": "Источник",
  "setup.provenanceScryer": "Scryer",
  "setup.sourcesLoading": "Загрузка клиентов и индексаторов…",
  "setup.duplicatesMerged": "Дубликаты из разных экземпляров объединены",
  "setup.summaryLibraries": "Библиотеки",
  "setup.summaryLibrariesDetail": "Фильмы, сериалы и аниме",
  "setup.summaryInstancesConnected": "Подключённые экземпляры",
  "setup.summaryInstancesConnectedDetail":
    "{{sonarr}} Sonarr · {{radarr}} Radarr",
  "setup.summaryRootsMapped": "Сопоставленные каталоги источников",
  "setup.summaryRootsMappedDetail": "Направлены в библиотеки Scryer",
  "setup.summaryPathsRemapped": "Переназначенные пути",
  "setup.summaryPathsRemappedDetail":
    "Исходные пути сохранены для корректного переноса состояния отслеживания",
  "setup.summaryClientsMerged": "Объединено между экземплярами",
  "setup.summaryIndexersEnabled": "Поддерживаются и включены",
  "setup.fetchingMonitoredStatus": "Получение состояния отслеживания…",
  "setup.fetchingMonitoredStatusDetail":
    "Scryer в фоновом режиме получает состояние отслеживания для каждого объекта из подключённых экземпляров.",
  "setup.monitoredStatusSynced": "Состояние отслеживания синхронизировано",
  "setup.monitoredStatusSyncedDetail":
    "Состояние отслеживания для всех объектов было получено из подключённых экземпляров.",
  "setup.importGateHint":
    "Импорт нельзя завершить, пока не завершится синхронизация состояния отслеживания.",
  "setup.importGateDoneHint": "Готово — теперь можно завершить импорт.",
  "setup.titlesFetched": "{{fetched}} / {{total}} объектов",
  "setup.importWarmupFailedTitle":
    "Не удалось синхронизировать состояние отслеживания",
  "setup.importWarmupFailedDetail":
    "Scryer не удалось получить состояние отслеживания из подключённых экземпляров. Проверьте соединения и повторите попытку.",
  "setup.importWarmupRetrying": "Перезапуск синхронизации…",
  "setup.importWarmupSessionExpiredTitle": "Подготовка к импорту истекла",
  "setup.importWarmupSessionExpired":
    "Подготовленные данные для импорта больше недоступны. Возможно, Scryer был перезапущен или процесс слишком долго оставался бездействующим. Повторно подключитесь к своим экземплярам, чтобы начать новый импорт.",
  "setup.importReconnect": "Подключиться снова",
  "setup.importUnmappedRootsNotice":
    "Некоторые обнаруженные каталоги источников ещё не привязаны к библиотеке. Вернитесь к шагу «Библиотеки» и сопоставьте их перед завершением импорта.",
  "settings.manageConfiguration": "Управление конфигурацией",
  "settings.runSetupWizard": "Запустить мастер настройки",
  "history.title": "История",
  "history.empty": "История событий пуста",
  "history.loadError": "Не удалось загрузить историю.",
  "history.retry": "Повторить",
  "history.event": "Событие",
  "history.titleColumn": "Название",
  "history.sourceTitle": "Исходное название",
  "history.quality": "Качество",
  "history.date": "Дата",
  "history.actor": "Пользователь",
  "history.filterByEventType": "Фильтр по типу события",
  "history.allEvents": "Все события",
  "history.grabbed": "Захвачено",
  "history.downloadFailed": "Ошибка загрузки",
  "history.blocklisted": "Заблокировано",
  "history.scanned": "Просканировано",
  "history.imported": "Импортировано",
  "history.importFailed": "Ошибка импорта",
  "history.importSkipped": "Импорт пропущен",
  "history.importRejectedByRule": "Отклонено правилом",
  "history.downloadCompleted": "Загружено",
  "history.fileUpgraded": "Обновлено",
  "history.fileRecycled": "Перемещено в корзину",
  "history.fileDeleted": "Удалено",
  "history.fileRestored": "Восстановлено",
  "history.fileRenamed": "Переименовано",
  "history.rematched": "Повторно сопоставлено",
  "history.downloadIgnored": "Проигнорировано",
  "history.downloadMissing": "Не видна в клиенте",
  "history.facet": "Тип",
  "history.actions": "Действия",
  "history.expandDetails": "Показать подробности события",
  "history.collapseDetails": "Скрыть подробности события",
  "history.collectionId": "Идентификатор коллекции",
  "history.episodeCountSingle": "1 эпизод",
  "history.episodeCountMultiple": "{{count}} эпизодов",
  "history.unknownEvent": "Неизвестно",
  "history.noMore": "Больше событий нет",
  "history.loadMore": "Загрузить ещё",
  "convenience.title": "Упрощённые правила",
  "convenience.requiredAudioLabel": "Обязательные языки аудиодорожек",
  "convenience.requiredAudioHelp":
    "Релизы без указанных языков аудио будут заблокированы. Будет создано управляемое правило, которое можно найти в разделе «Настройки → Правила».",
  "settings.autoBackupsTitle": "Автоматические резервные копии",
  "settings.autoBackupsDisabledMissingKeyToastTitle":
    "Автоматическое резервное копирование отключено",
  "settings.autoBackupsDisabledMissingKeyToastDescription":
    "Теперь для автоматического резервного копирования требуется сохранённый ключ. Откройте раздел «Резервные копии», задайте ключ и снова включите эту функцию.",
  "settings.autoBackupsDisabledMissingKeyToastAction":
    "Открыть резервные копии",
  "settings.autoBackupsEnabledHelp":
    "Этот планировщик действует для всей системы и запускается один раз в день в указанное время.",
  "settings.autoBackupsTime": "Время ежедневного резервного копирования",
  "settings.autoBackupsHour": "Часы",
  "settings.autoBackupsMinute": "Минуты",
  "settings.autoBackupsNextRun":
    "Следующее автоматическое резервное копирование",
  "settings.autoBackupsKeyLabel": "Ключ автоматического резервного копирования",
  "settings.autoBackupsKeyHelpLabel":
    "О ключе автоматического резервного копирования",
  "settings.autoBackupsSetKey":
    "Установить ключ автоматического резервного копирования",
  "settings.autoBackupsReplaceKey":
    "Заменить ключ автоматического резервного копирования",
  "settings.autoBackupsKeyAlreadySetHint":
    "(ключ резервного копирования уже задан)",
  "settings.autoBackupsKeyPresent":
    "Ключ автоматического резервного копирования сохранён.",
  "settings.autoBackupsKeyAbsent":
    "Ключ автоматического резервного копирования не сохранён. Автоматическое резервное копирование невозможно без настроенного ключа.",
  "settings.autoBackupsKeySummary":
    "Обязательное шифрование автоматических резервных копий",
  "settings.autoBackupsKeyHelp":
    "Оставьте поле пустым, чтобы сохранить текущий ключ. Введите новое значение длиной не менее {{count}} непробельных символов, чтобы заменить существующий ключ.",
  "settings.autoBackupsKeyRequired":
    "Для автоматического резервного копирования требуется сохранённый ключ длиной не менее {{count}} непробельных символов.",
  "settings.autoBackupsKeyTooShort":
    "Ключ автоматического резервного копирования должен содержать не менее {{count}} непробельных символов.",
  "settings.autoBackupsShowKey":
    "Показать ключ автоматического резервного копирования",
  "settings.autoBackupsHideKey":
    "Скрыть ключ автоматического резервного копирования",
  "settings.autoBackupsSaved":
    "Настройки автоматического резервного копирования сохранены.",
  "settings.backupLocationTitle": "Расположение резервных копий",
  "settings.backupLocationCustomPath": "Пользовательский путь",
  "settings.backupLocationBrowse": "Обзор",
  "settings.backupLocationEffectivePath": "Активный путь",
  "settings.backupLocationReset": "Использовать значение по умолчанию",
  "settings.backupLocationPickerTitle": "Выберите папку для резервных копий",
  "settings.backupLocationSaved": "Расположение резервных копий сохранено.",
  "settings.backupsSection":
    "Создавайте резервные копии Scryer, которые можно использовать для восстановления. Ручное резервное копирование выполняется в фоновом режиме, поэтому вы можете продолжать работу, даже если автоматическое резервное копирование уже запущено.",
  "settings.backupsEmpty": "Резервные копии ещё не созданы.",
  "settings.backupsCreate": "Создать резервную копию",
  "settings.backupsCreateTitle": "Создать архив резервной копии",
  "settings.backupsCreateDescription":
    "Защитите эту резервную копию паролем. Этот же пароль потребуется при восстановлении.",
  "settings.backupsConfirmPassword": "Подтвердите пароль",
  "settings.backupsPasswordMismatch": "Пароли не совпадают",
  "settings.backupsQueued":
    "Создание резервной копии запущено в фоновом режиме.",
  "settings.backupsDeleted": "Резервная копия удалена.",
  "settings.backupsCreating": "Создание",
  "settings.backupsReady": "Готово",
  "settings.backupsInvalid": "Недействительно",
  "settings.backupsFailed": "Ошибка",
  "settings.backupsEncrypted": "Зашифровано",
  "settings.backupsPlaintext": "Без шифрования",
  "settings.backupsManual": "Вручную",
  "settings.backupsAutomatic": "Автоматически",
  "settings.backupsDownload": "Скачать",
  "settings.backupsDelete": "Удалить",
  "settings.backupsDeleteConfirm":
    "Удалить эту резервную копию с сервера? Загруженные копии затронуты не будут.",
  "settings.unsavedBackupChangesTitle":
    "Несохранённые настройки резервного копирования",
  "settings.unsavedBackupChangesConfirm":
    "У вас есть несохранённые изменения в настройках резервного копирования. Выйти без сохранения?",
  "settings.backupsTables": "{{count}} таблиц",
  "settings.backupsRows": "{{count}} строк",
  "settings.recycleBin": "Корзина",
  "settings.recycleBinEnabled": "Включить корзину",
  "settings.recycleBinEnabledHelp":
    "Удалённые и заменённые медиафайлы сначала перемещаются в корзину, а затем окончательно удаляются.",
  "settings.recycleBinEnabledReadonly":
    "Состояние корзины может изменяться только пользователями с правами управления конфигурацией.",
  "settings.recycleBinSection": "Управление удалёнными медиафайлами",
  "settings.recycleBinLibraryFilter": "Библиотеки",
  "settings.recycleBinEmpty": "Корзина пуста.",
  "settings.recycleBinNoManageableLibraries":
    "Нет доступных библиотек для управления содержимым корзины.",
  "settings.recycleBinRestore": "Восстановить",
  "settings.recycleBinDelete": "Удалить",
  "settings.recycleBinEmptyAll": "Очистить корзину",
  "settings.recycleBinEmptyConfirm":
    "Безвозвратно удалить {{count}} элементов из текущего представления корзины? Это действие нельзя отменить.",
  "settings.recycleBinDeleteConfirm":
    "Безвозвратно удалить этот файл? Это действие нельзя отменить.",
  "settings.recycleBinReason": "Причина",
  "settings.recycleBinSize": "Размер",
  "settings.recycleBinRecycled": "Перемещено в корзину",
  "settings.recycleBinFilterPlaceholder": "Фильтр по имени файла или названию",
  "settings.recycleBinFilterAria":
    "Фильтровать файлы в корзине по имени файла или названию",
  "settings.recycleBinSelectedCount": "Выбрано: {{count}}",
  "settings.recycleBinRestoreSelected": "Восстановить выбранные",
  "settings.recycleBinDeleteSelected": "Удалить выбранные",
  "settings.recycleBinClearSelection": "Снять выделение",
  "settings.recycleBinNoFilterMatches":
    "Файлы в корзине, соответствующие этому фильтру, не найдены.",
  "settings.recycleBinSelectAllFiltered":
    "Выбрать все отфильтрованные файлы в корзине",
  "settings.recycleBinSelectTitle": "Выбрать {{title}}",
  "settings.recycleBinSelectFile": "Выбрать {{name}}",
  "settings.recycleBinUnassociatedFiles": "Несвязанные файлы",
  "settings.recycleBinFileCountOne": "{{count}} файл",
  "settings.recycleBinFileCountOther": "{{count}} файлов",
  "settings.recycleBinBatchLimit":
    "Можно выбрать не более {{count}} файлов из корзины одновременно.",
  "settings.recycleBinRestoreQueuedOne":
    "Восстановление {{count}} файла из корзины поставлено в очередь.",
  "settings.recycleBinRestoreQueuedOther":
    "Восстановление {{count}} файлов из корзины поставлено в очередь.",
  "settings.recycleBinDeleteQueuedOne":
    "Удаление {{count}} файла из корзины поставлено в очередь.",
  "settings.recycleBinDeleteQueuedOther":
    "Удаление {{count}} файлов из корзины поставлено в очередь.",
  "settings.recycleBinRestoreSelectedTitle": "Восстановить файлы из корзины",
  "settings.recycleBinRestoreOccupiedOne":
    "{{count}} место назначения уже занято. Выберите, сохранить ли оба файла или переместить текущий файл в корзину перед восстановлением.",
  "settings.recycleBinRestoreOccupiedOther":
    "{{count}} мест назначения уже заняты. Выберите, сохранить ли оба файла или переместить текущие файлы в корзину перед восстановлением.",
  "settings.recycleBinRestoreDescriptionOne":
    "Восстановить {{count}} выбранный файл в исходное расположение.",
  "settings.recycleBinRestoreDescriptionOther":
    "Восстановить {{count}} выбранных файлов в исходные расположения.",
  "settings.recycleBinDeleteSelectedConfirmOne":
    "Безвозвратно удалить {{count}} выбранный файл из корзины? Это действие нельзя отменить.",
  "settings.recycleBinDeleteSelectedConfirmOther":
    "Безвозвратно удалить {{count}} выбранных файлов из корзины? Это действие нельзя отменить.",
  "settings.recycleBinKeepBothAndRestore": "Сохранить оба и восстановить",
  "settings.recycleBinReplaceExistingAndRestore":
    "Заменить существующий и восстановить",
  "settings.recycleBinKeepBoth": "Сохранить оба",
  "settings.recycleBinKeepBothDescription":
    "Конфликтующие файлы будут восстановлены с суффиксом -restored.",
  "settings.recycleBinReplaceExisting": "Заменить существующий",
  "settings.recycleBinReplaceExistingDescription":
    "Переместить текущий файл в корзину, а затем восстановить выбранный файл на его место.",
  "status.recycleBinRestored": "Файл восстановлен в {{path}}.",
  "status.recycleBinDeleted": "Файл безвозвратно удалён.",
  "status.recycleBinQuarantined":
    "Запись была помещена в карантин вместо удаления — файл небезопасно удалять.",
  "status.recycleBinEmptied": "Корзина очищена (удалено {{count}} элементов).",
  "status.recycleBinSettingsSaved": "Настройки корзины сохранены.",
  "settings.recycleBinPathExistingItems":
    "При смене папки элементы, уже находящиеся в корзине, перемещаются в новую папку. Элемент, который не удалось переместить, остаётся в прежней папке и будет показан здесь после сохранения.",
  "settings.recycleBinRelocatedOne":
    "Элементов корзины перемещено в новую папку: {{count}}.",
  "settings.recycleBinRelocatedOther":
    "Элементов корзины перемещено в новую папку: {{count}}.",
  "settings.recycleBinRelocationFailedOne":
    "Не удалось переместить элементы корзины ({{count}}):",
  "settings.recycleBinRelocationFailedOther":
    "Не удалось переместить элементы корзины ({{count}}):",
  "status.recycleBinSettingsSavedWithRelocationFailures":
    "Настройки корзины сохранены, но некоторые элементы не удалось переместить.",
  "form.apiKeyStoredPlaceholder": "Ключ API сохранён",
  "appUpgrade.title": "Обновление приложения",
  "appUpgrade.subtitle": "Версия и управление обновлениями",
  "appUpgrade.upgradeNow": "Обновить сейчас",
  "appUpgrade.confirmTitle": "Обновить приложение?",
  "appUpgrade.confirmDescription": "Обновить до {{version}}?",
  "appUpgrade.confirmAction": "Начать обновление",
  "appUpgrade.restartNotice": "Scryer будет перезапущен во время обновления.",
  "appUpgrade.uacNotice": "Windows покажет локальный запрос повышения прав.",
  "appUpgrade.startFailed": "Не удалось начать обновление.",
  "appUpgrade.targetChanged":
    "Доступное обновление изменилось. Проверьте его и повторите попытку.",
  "appUpgrade.loading": "Загрузка статуса обновления…",
  "appUpgrade.currentVersion": "Текущая версия",
  "appUpgrade.availableVersion": "Доступная версия",
  "appUpgrade.noUpdate": "Нет доступных обновлений",
  "appUpgrade.installationKind": "Тип установки",
  "appUpgrade.managementOwner": "Управление обновлением",
  "appUpgrade.owner.inApp": "Scryer",
  "appUpgrade.owner.operator": "Оператор",
  "appUpgrade.eligibility": "Возможность обновления",
  "appUpgrade.eligibility.eligible":
    "Эту установку можно обновить в приложении.",
  "appUpgrade.eligibility.managed_by_docker":
    "Эта установка управляется Docker.",
  "appUpgrade.eligibility.managed_by_homebrew":
    "Эта установка управляется Homebrew.",
  "appUpgrade.eligibility.managed_by_winget":
    "Эта установка управляется winget.",
  "appUpgrade.eligibility.windows_supervised":
    "Эта установка Windows управляется извне.",
  "appUpgrade.eligibility.disabled_by_operator":
    "Обновления в приложении отключены оператором.",
  "appUpgrade.eligibility.unsupported_layout":
    "Этот тип установки не поддерживается.",
  "appUpgrade.eligibility.install_dir_not_writable":
    "Каталог установки недоступен для записи.",
  "appUpgrade.eligibility.app_bundle_translocated":
    "macOS запускает Scryer из временной копии. Переместите Scryer в папку «Программы» и откройте его оттуда.",
  "appUpgrade.eligibility.app_bundle_read_only_volume":
    "Scryer запущен с тома, доступного только для чтения. Перетащите Scryer в папку «Программы» и откройте его оттуда.",
  "appUpgrade.eligibility.app_bundle_not_tray_supervised":
    "Это приложение запущено не приложением Scryer, поэтому его нельзя перезапустить после обновления.",
  "appUpgrade.eligibility.unknown":
    "Сведения о возможности обновления недоступны.",
  "appUpgrade.rebootRequired":
    "Перезапустите Windows, чтобы завершить обновление.",
  "appUpgrade.activeRun": "Идёт обновление",
  "appUpgrade.latestRun": "Последнее обновление",
  "appUpgrade.runStatusLabel": "Статус: {{status}}",
  "appUpgrade.runStatus.queued": "В очереди",
  "appUpgrade.runStatus.discovering": "Подготовка",
  "appUpgrade.runStatus.running": "Выполняется",
  "appUpgrade.runStatus.completed": "Завершено",
  "appUpgrade.runStatus.warning": "Завершено с предупреждениями",
  "appUpgrade.runStatus.failed": "Не удалось",
  "appUpgrade.phase.checking": "Проверка",
  "appUpgrade.phase.downloading": "Загрузка",
  "appUpgrade.phase.verifying": "Проверка",
  "appUpgrade.phase.staging": "Подготовка",
  "appUpgrade.phase.applying": "Применение",
  "appUpgrade.phase.awaiting_elevation": "Ожидание повышения прав",
  "appUpgrade.phase.restarting": "Перезапуск",
  "appUpgrade.phase.reboot_required": "Требуется перезапуск",
  "appUpgrade.phase.unknown": "Выполняется",
  "appUpgrade.downloadProgress": "Загружено {{downloaded}} из {{total}}",
  "appUpgrade.downloadedBytes": "Загружено {{downloaded}}",
  "appUpgrade.operatorGuidance": "Обновление управляется оператором",
  "appUpgrade.guidance.winget":
    "Выполните эту команду в терминале с повышенными правами.",
  "appUpgrade.guidance.wingetCommand": "winget upgrade ScryerMedia.Scryer",
  "appUpgrade.guidance.docker":
    "Загрузите новый образ и пересоздайте контейнер с вашей конфигурацией развёртывания.",
  "appUpgrade.guidance.homebrew": "Выполните эту команду в терминале.",
  "appUpgrade.guidance.homebrewCommand": "brew upgrade scryer",
  "appUpgrade.guidance.disabled":
    "Обновления в приложении отключены переменной окружения.",
  "appUpgrade.guidance.disabledVariable": "SCRYER_DISABLE_SELF_UPGRADE",
  "appUpgrade.guidance.other":
    "Используйте способ обновления, предоставленный оператором этой установки.",
  "appUpgrade.installationKind.portable": "Портативная",
  "appUpgrade.installationKind.directMsi": "Прямой MSI",
  "appUpgrade.installationKind.macosAppBundle": "Приложение macOS",
  "appUpgrade.installationKind.docker": "Docker",
  "appUpgrade.installationKind.homebrew": "Homebrew",
  "appUpgrade.installationKind.winget": "winget",
  "appUpgrade.installationKind.windowsSupervised": "Windows под управлением",
  "appUpgrade.installationKind.disabled": "Отключено",
  "appUpgrade.installationKind.unsupported": "Не поддерживается",
  "appUpgrade.success": "Обновлено до {{version}}",
  "appUpgrade.failed": "Не удалось обновить приложение.",
  "settings.maintenanceRules": "Правила обслуживания",
  "nav.rules": "Правила",
  "settings.rulesScoring": "Правила оценки",
  "settings.maintenanceNavRules": "Правила",
  "settings.maintenanceNavHistory": "История",
  "settings.maintenanceRuleAction": "Действие",
  "settings.maintenanceRuleActionHelp":
    "Что сделало бы это правило, когда действия обслуживания будут включены.",
  "settings.maintenanceRuleActionPlaceholder": "Выберите действие",
  "settings.maintenanceRuleNoActions": "Действия уровня тайтла недоступны.",
  "settings.maintenanceRuleGraceDays": "Дни отсрочки",
  "settings.maintenanceRuleGraceDaysHelp":
    "Дни после первого совпадения правила до момента, когда выполнилось бы действие (действия пока не выполняются).",
  "settings.maintenanceRuleTargetProfile": "Целевой профиль качества",
  "settings.maintenanceRuleTargetProfileHelp":
    "Профиль, на который это действие перевело бы совпавшие тайтлы.",
  "settings.maintenanceRuleTargetProfileIdHelp":
    "Идентификатор профиля качества.",
  "settings.maintenanceRuleLibraries": "Область библиотек",
  "settings.maintenanceRuleLibrariesHelp":
    "Не отмечайте ни одной библиотеки, чтобы правило действовало на всю коллекцию.",
  "settings.maintenanceRuleLibrariesAll": "Все библиотеки",
  "settings.maintenanceRuleCreate": "Создать правило обслуживания",
  "settings.maintenanceRuleUpdate": "Обновить правило обслуживания",
  "settings.maintenanceRuleCopy": "Скопировать как новое правило",
  "settings.maintenanceRuleCreateNew": "Создать новое правило обслуживания",
  "settings.maintenanceRuleValidationRequired":
    "Имя, исходный код правила и действие обязательны.",
  "settings.maintenanceRuleConfirmDiscardTitle":
    "Отменить несохранённые изменения?",
  "settings.maintenanceRuleConfirmDiscardDescription":
    "В редакторе правил обслуживания есть несохранённые изменения, которые будут потеряны.",
  "settings.maintenanceRuleDeleting": "Удалить правило обслуживания {{name}}?",
  "settings.noMaintenanceRulesFound": "Правил обслуживания пока нет.",
  "settings.maintenanceRuleRevision": "Ревизия",
  "settings.maintenanceRuleRisk": "Риск",
  "settings.maintenanceRiskNone": "Без риска",
  "settings.maintenanceRiskLow": "Низкий риск",
  "settings.maintenanceRiskMedium": "Средний риск",
  "settings.maintenanceRiskHigh": "Высокий риск",
  "settings.maintenanceActionDoNothing": "Ничего не делать",
  "settings.maintenanceActionUnmonitorScopeKeepFiles":
    "Снять отслеживание области, сохранить файлы",
  "settings.maintenanceActionDeleteTitleAndFiles": "Удалить тайтл и его файлы",
  "settings.maintenanceActionUnmonitorTitleDeleteAllFiles":
    "Снять отслеживание тайтла, удалить все файлы",
  "settings.maintenanceActionUnmonitorShowDeleteExistingFiles":
    "Снять отслеживание сериала, удалить существующие файлы",
  "settings.maintenanceActionUnmonitorScopeDeleteFiles":
    "Снять отслеживание области, удалить её файлы",
  "settings.maintenanceActionUnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty":
    "Снять отслеживание сезона, удалить его файлы, затем удалить сериал, если он пуст",
  "settings.maintenanceActionUnmonitorSeasonThenUnmonitorShowIfEmpty":
    "Снять отслеживание сезона, затем снять отслеживание сериала, если он пуст",
  "settings.maintenanceActionChangeQualityProfileAndSearchIfChanged":
    "Изменить профиль качества и выполнить поиск, если он изменился",
  "settings.maintenancePreviewTitle": "Предпросмотр",
  "settings.maintenancePreviewSubtitle":
    "Запустите правило на реальных тайтлах. Предпросмотр только читает: он сообщает, что совпало бы, и ничего не меняет.",
  "settings.maintenancePreviewSource": "Правило",
  "settings.maintenancePreviewSourceStored": "Сохранённое правило",
  "settings.maintenancePreviewSourceDraft": "Черновик редактора",
  "settings.maintenancePreviewLibrary": "Библиотека",
  "settings.maintenancePreviewLibraryPlaceholder": "Выберите библиотеку",
  "settings.maintenancePreviewLimit": "Тайтлы",
  "settings.maintenancePreviewLimitHelp": "До {{max}} тайтлов за прогон.",
  "settings.maintenancePreviewRun": "Запустить предпросмотр",
  "settings.maintenancePreviewRunning": "Выполняется предпросмотр...",
  "settings.maintenancePreviewEmpty":
    "Выберите библиотеку и запустите предпросмотр.",
  "settings.maintenancePreviewNoTitles": "Ни один тайтл не был оценён.",
  "settings.maintenancePreviewColOutcome": "Результат",
  "settings.maintenancePreviewColReasons": "Коды причин",
  "settings.maintenancePreviewOutcomeMatch": "СОВПАЛО",
  "settings.maintenancePreviewOutcomeNoMatch": "НЕ СОВПАЛО",
  "settings.maintenancePreviewOutcomeUnknown": "НЕИЗВЕСТНО",
  "settings.maintenancePreviewOutcomeError": "ОШИБКА",
  "settings.maintenancePreviewMeta": "Оценено {{time}} · правило {{hash}}",
  "settings.maintenancePreviewRuleSelect": "Правило",
  "settings.maintenancePreviewRuleSelectPlaceholder":
    "Выберите сохранённое правило",
  "settings.maintenanceRuleUpdated": "Правило обслуживания обновлено.",
  "settings.maintenanceRuleCreated":
    "Правило обслуживания создано и сохранено отключённым.",
  "settings.maintenanceRuleDeleted": "Правило обслуживания {{name}} удалено.",
  "settings.maintenanceArmingNone": "Без эффектов",
  "settings.maintenanceArmingReversible": "Обратимое",
  "settings.maintenanceArmingDestructive": "Разрушительное",
  "settings.maintenanceArmDestructiveLoading":
    "Подсчёт кандидатов, которые уже удерживает это правило…",
  "settings.maintenanceArmDestructiveCount":
    "Действие может затронуть {{count}} кандидатов.",
  "settings.maintenanceArmDestructiveNoCandidates":
    "Сейчас у этого правила нет кандидатов, которых могло бы затронуть действие.",
  "settings.maintenanceArmDestructiveCountChanged":
    "Пока вы читали, число изменилось: теперь правило удерживает {{count}}. Проверьте число и подтвердите ещё раз.",
  "settings.maintenanceArmDestructiveAcknowledge":
    "Я понимаю, что это правило может удалить файлы {{count}} тайтлов.",
  "settings.maintenanceCandidatesFilterRule": "Правило",
  "settings.maintenanceCandidatesFilterState": "Состояние",
  "settings.maintenanceCandidatesFilterLibrary": "Библиотека",
  "settings.maintenanceCandidatesAllRules": "Все правила",
  "settings.maintenanceCandidatesAllStates": "Все состояния",
  "settings.maintenanceCandidatesAllLibraries": "Все библиотеки",
  "settings.maintenanceCandidatesColRule": "Правило",
  "settings.maintenanceCandidatesColState": "Состояние",
  "settings.maintenanceCandidatesColFirstMatched": "Первое совпадение",
  "settings.maintenanceCandidatesColDue": "Срок",
  "settings.maintenanceCandidatesHeldSince": "Удерживается с {{time}}",
  "settings.maintenanceCandidatesEmpty":
    "Ни один кандидат не подходит под эти фильтры.",
  "settings.maintenanceCandidateExclude": "Исключить этот тайтл",
  "settings.maintenanceCandidateStateObserving": "Наблюдение",
  "settings.maintenanceCandidateStatePendingAction": "Ожидает действия",
  "settings.maintenanceCandidateStateDue": "Срок наступил",
  "settings.maintenanceCandidateStateExecuting": "Выполняется",
  "settings.maintenanceCandidateStateSucceeded": "Успешно",
  "settings.maintenanceCandidateStateFailed": "Не удалось",
  "settings.maintenanceCandidateStateCanceled": "Отменено",
  "settings.maintenanceCandidateStateExcluded": "Исключено",
  "settings.maintenanceCandidateStateBlocked": "Заблокировано",
  "settings.maintenanceCountdownDueNow": "Срок наступил",
  "settings.maintenanceCountdownInMinutes": "через {{count}} мин",
  "settings.maintenanceCountdownInHours": "через {{count}} ч",
  "settings.maintenanceCountdownInDays": "через {{count}} дн.",
  "settings.maintenanceCountdownOverdueMinutes": "просрочено на {{count}} мин",
  "settings.maintenanceCountdownOverdueHours": "просрочено на {{count}} ч",
  "settings.maintenanceCountdownOverdueDays": "просрочено на {{count}} дн.",
  "settings.maintenanceRunsTitle": "Запуски и история",
  "settings.maintenanceRunsSubtitle":
    "Недавние проходы оценки и попытки действий, сначала новые.",
  "settings.maintenanceRunScope": "Охват",
  "settings.maintenanceRunScopeAll": "Все правила",
  "settings.maintenanceRunEvaluationNow": "Оценить сейчас",
  "settings.maintenanceRunEvaluationRunning": "Идёт оценка…",
  "settings.maintenanceRunActionsNow": "Выполнить действия сейчас",
  "settings.maintenanceRunActionsRunning": "Выполняются действия…",
  "settings.maintenanceRunStarted": "Запуск начат.",
  "settings.maintenanceRunNotStarted": "Ничего не запущено: {{message}}",
  "settings.maintenanceEvaluationRunsTitle": "Запуски оценки",
  "settings.maintenanceEvaluationRunsEmpty":
    "Запусков оценки пока не записано.",
  "settings.maintenanceActionRunsTitle": "Запуски действий",
  "settings.maintenanceActionRunsEmpty": "Запусков действий пока не записано.",
  "settings.maintenanceRunColStarted": "Начат",
  "settings.maintenanceRunColStatus": "Статус",
  "settings.maintenanceRunColCounts": "Счётчики",
  "settings.maintenanceRunColDuration": "Длительность",
  "settings.maintenanceRunColError": "Ошибка",
  "settings.maintenanceRunCounts":
    "оценено {{evaluated}} · совпало {{matched}} · без совпадения {{noMatch}} · неизвестно {{unknown}} · ошибок {{errors}}",
  "settings.maintenanceRunDuration": "{{seconds}} с",
  "settings.maintenanceRunRevision": "рев. {{revision}}",
  "settings.maintenanceRunStatusRunning": "Выполняется",
  "settings.maintenanceRunStatusSucceeded": "Успешно",
  "settings.maintenanceRunStatusFailed": "Не удалось",
  "settings.maintenanceRunStatusHeld": "Удержано",
  "settings.maintenanceRunStatusAlreadySatisfied": "Уже выполнено",
  "settings.maintenanceRunStatusSkipped": "Пропущено",
  "settings.maintenanceActionRunColAttempt": "Попытка",
  "settings.maintenanceActionRunColHoldReason": "Причина удержания",
  "settings.maintenanceExclusionsTitle": "Исключения",
  "settings.maintenanceExclusionsSubtitle":
    "Тайтлы, которых правило обслуживания не должно касаться. Глобальное исключение действует для всех правил.",
  "settings.maintenanceExclusionsEmpty": "Исключений нет.",
  "settings.maintenanceExclusionColScope": "Охват",
  "settings.maintenanceExclusionColReason": "Причина",
  "settings.maintenanceExclusionColCreated": "Добавлено",
  "settings.maintenanceExclusionScopeGlobal": "Все правила",
  "settings.maintenanceExclusionRemove": "Удалить исключение",
  "settings.maintenanceExclusionRemoved": "Исключение для {{title}} удалено.",
  "settings.maintenanceExclusionRemoving": "Удалить исключение для {{title}}?",
  "settings.maintenanceExcludeTitle": "Исключить из обслуживания",
  "settings.maintenanceExcludeBody":
    "{{title}} будет исключён начиная со следующей оценки, а его кандидат перейдёт в состояние «Исключено».",
  "settings.maintenanceExcludeScope": "Охват",
  "settings.maintenanceExcludeScopeGlobal": "Все правила",
  "settings.maintenanceExcludeReason": "Причина",
  "settings.maintenanceExcludeReasonPlaceholder":
    "Почему этот тайтл исключается (необязательно)",
  "settings.maintenanceExcludeConfirm": "Исключить",
  "settings.maintenanceExcluded": "{{title}} исключён из обслуживания.",
  "settings.refMaintTitle": "Справочник входных данных обслуживания",
  "settings.refMaintSubtitle":
    "Нажмите, чтобы развернуть полный список фактов, доступных правилу обслуживания.",
  "settings.refMaintIntro":
    "Каждое правило обслуживания получает один объект input на субъект: input.subject определяет, что оценивается, input.library — где он находится, input.facts содержит снимок фактов, а сам input несёт конверт оценки. Факт — это обычное значение, а ключ просто отсутствует, когда сообщать нечего, поэтому пишите input.facts.monitored, а не проверку статуса. Если правило читает факт, который Scryer не смог наблюдать для этого субъекта, Scryer сам удержит субъект — писать такую защиту вручную не нужно.",
  "settings.refMaintOutputTitle": "Результат правила",
  "settings.refMaintOutputIntro":
    "Правило определяет не более трёх имён. Всё остальное в пакете игнорируется, а решение, не соответствующее в точности этой форме, отклоняется, а не приводится к типу.",
  "settings.refMaintOutputMatch":
    "match — логическое значение. Если оно не определено, субъект не совпал.",
  "settings.refMaintOutputUnknown":
    "unknown — логическое значение. Устанавливайте его, когда само правило не может принять решение; unknown всегда имеет приоритет над совпадением. Факты, которые Scryer не смог наблюдать, уже обрабатываются автоматически.",
  "settings.refMaintOutputReasons":
    "reasons — набор коротких машинных кодов, объясняющих решение. Они сохраняются у кандидата и показываются в предпросмотре.",
  "settings.refMaintOutputNoPackage":
    "Не пишите строки package или import. Scryer добавляет их при сохранении правила и удаляет обратно для редактора.",
  "settings.refMaintSectionInput": "— Конверт оценки",
  "settings.refMaintInputSchemaVersion":
    "Версия документа входных данных обслуживания, под которую написано правило",
  "settings.refMaintInputEvaluationTime":
    "Момент времени в формате RFC3339, фиксируемый один раз за прогон, чтобы все субъекты сравнивались с одними и теми же часами. Используйте его для вычислений с датами: time.now_ns() существует, но тогда один и тот же субъект при повторе решался бы иначе.",
  "settings.refMaintSectionSubject": "— Оцениваемый тайтл, сезон или эпизод",
  "settings.refMaintSubjectKind": "Уровень оценки: тайтл, сезон или эпизод",
  "settings.refMaintSubjectTitleId":
    "Идентификатор Scryer для тайтла, к которому относится субъект",
  "settings.refMaintSubjectSeasonNumber":
    "Номер сезона; присутствует только для субъектов-сезонов",
  "settings.refMaintSubjectEpisodeId":
    "Идентификатор эпизода; присутствует только для субъектов-эпизодов",
  "settings.refMaintSubjectFacet": "Медиа-фасет тайтла (фильм, сериал, аниме)",
  "settings.refMaintSubjectName": "Отображаемое название тайтла",
  "settings.refMaintSubjectYear": "Год выпуска тайтла, если он известен",
  "settings.refMaintSectionLibrary":
    "— Библиотека, в которой находится субъект",
  "settings.refMaintLibraryId": "Идентификатор библиотеки, содержащей субъект",
  "settings.refMaintLibraryName": "Отображаемое имя этой библиотеки",
  "settings.refMaintSectionFacts":
    "— Снимок фактов; каждый факт — обычное значение, а ключ отсутствует, если факт отсутствует или его не удалось наблюдать",
  "settings.refMaintFactsMonitored": "Отслеживается ли субъект",
  "settings.refMaintFactsTags":
    "Пользовательские теги субъекта в нижнем регистре из реестра тегов, который администратор ведёт в настройках; служебные записи с префиксом scryer: исключены",
  "settings.refMaintFactsQualityProfileId":
    "Профиль качества, назначенный субъекту",
  "settings.refMaintFactsAddedAt": "Когда субъект был добавлен в библиотеку",
  "settings.refMaintFactsAddedByUserId":
    "Какой пользователь добавил субъект; отсутствует, если его создало сканирование или обнаружение",
  "settings.refMaintFactsAddedByUsername":
    "Имя пользователя, добавившего субъект",
  "settings.refMaintFactsRequested": "Был ли субъект создан по медиа-запросу",
  "settings.refMaintFactsRequestedByUserIds":
    "Пользователи, запросившие субъект: автор запроса и все дополнительные запросившие",
  "settings.refMaintFactsRequestedByUsernames":
    "Имена этих запросивших пользователей",
  "settings.refMaintFactsFirstImportedAt":
    "Когда для субъекта был впервые импортирован файл",
  "settings.refMaintFactsLastUpgradedAt":
    "Когда субъект в последний раз получал улучшение",
  "settings.refMaintFactsHasFile":
    "Есть ли у субъекта хотя бы один файл на диске",
  "settings.refMaintFactsFileCount":
    "Сколько файлов субъекта находится на диске",
  "settings.refMaintFactsTotalFileSizeBytes":
    "Суммарный размер файлов субъекта на диске",
  "settings.refMaintFactsFiles":
    "Файлы субъекта с подробностями по каждому файлу",
  "settings.refMaintFactsEpisodeCount": "Сколько эпизодов охватывает субъект",
  "settings.refMaintFactsEpisodeFileCount":
    "Сколько из этих эпизодов имеют файл",
  "settings.refMaintFactsMonitoredEpisodeCount":
    "Сколько из этих эпизодов отслеживается",
  "settings.refMaintFactsActiveDownloads":
    "Есть ли у субъекта активная загрузка",
  "settings.refMaintFactsWatchedByUserIds":
    "Пользователи, которые смотрели субъект на подключённом медиасервере",
  "settings.refMaintFactsLastWatchedAt":
    "Когда субъект в последний раз кто-либо смотрел",
  "settings.refMaintFactsWatchedByAnyRequester":
    "Смотрел ли субъект хотя бы один из запросивших его пользователей",
  "settings.refMaintFactsWatchedByAllRequesters":
    "Смотрели ли субъект все запросившие его пользователи",
  "settings.refMaintFactsRequestLeaseState":
    "Сводное состояние сроков хранения по запросам: none, dormant, active или expired",
  "settings.refMaintFactsRequestLeaseExpiresAt":
    "Когда истекает последний действующий срок хранения субъекта",
  "settings.refMaintFactsActiveRetentionClaims":
    "Сколько сроков хранения по запросам всё ещё удерживают субъект",
  "settings.refMaintObsStatus":
    "known, absent или unknown. absent означает, что источник ответил и значения нет; unknown означает, что Scryer не смог выяснить. Проверяйте это перед чтением value — неизвестный факт никогда не приводится к false, 0 или пустой строке.",
  "settings.refMaintSectionObservations":
    "— Те же факты в виде полных конвертов, для правил, которым нужно отличать absent от unknown или читать причину. Чтение факта здесь отключает для него автоматическое удержание: ответственность за трёхзначный ответ берёте на себя вы.",
  "settings.refMaintSectionObsFiles":
    "— По одной записи на файл в input.observations.files.value",
  "settings.refMaintObsObservedAt":
    "Когда значение было получено, если источник записал время",
  "settings.refMaintObsReason":
    "Устойчивый машинный код, объясняющий, почему факт неизвестен или почему у отсутствующего факта нет значения",
  "settings.refMaintFactsMonitoredValue": "True, когда субъект отслеживается",
  "settings.refMaintFactsTagsValue":
    "Имена пользовательских тегов в нижнем регистре из реестра тегов администратора; никогда не служебные записи с префиксом scryer:",
  "settings.refMaintFactsQualityProfileIdValue":
    "Идентификатор назначенного профиля качества",
  "settings.refMaintFactsAddedAtValue":
    "Метка времени RFC3339 добавления субъекта",
  "settings.refMaintFactsAddedByUserIdValue":
    "Идентификатор пользователя, добавившего субъект; причина title_added_by_system, если этого не сделал никто",
  "settings.refMaintFactsAddedByUsernameValue":
    "Имя этого пользователя; unknown с причиной user_not_found, если учётной записи больше нет",
  "settings.refMaintFactsRequestedValue":
    "True, когда субъект создан хотя бы одним медиа-запросом; в противном случае — подтверждённое false",
  "settings.refMaintFactsRequestedByUserIdsValue":
    "Идентификаторы запросивших без повторов; пустой список, если субъект не запрашивали",
  "settings.refMaintFactsRequestedByUsernamesValue":
    "Имена запросивших; unknown с причиной user_not_found, если хотя бы одной из этих учётных записей больше нет",
  "settings.refMaintFactsFirstImportedAtValue":
    "Метка времени RFC3339 первого импорта",
  "settings.refMaintFactsLastUpgradedAtValue":
    "Метка времени RFC3339 последнего улучшения",
  "settings.refMaintFactsHasFileValue":
    "True, когда на диске есть хотя бы один файл",
  "settings.refMaintFactsFileCountValue": "Количество файлов на диске",
  "settings.refMaintFactsTotalFileSizeBytesValue":
    "Суммарный размер этих файлов в байтах",
  "settings.refMaintFactsFilesValue":
    "По одной записи на файл; см. input.observations.files.value[] ниже",
  "settings.refMaintFactsEpisodeCountValue":
    "Количество эпизодов в области действия",
  "settings.refMaintFactsEpisodeFileCountValue":
    "Количество этих эпизодов, у которых есть файл",
  "settings.refMaintFactsMonitoredEpisodeCountValue":
    "Количество этих эпизодов, которые отслеживаются",
  "settings.refMaintFactsActiveDownloadsValue":
    "True, когда для субъекта выполняется загрузка",
  "settings.refMaintFactsWatchedByUserIdsValue":
    "Идентификаторы всех, кто смотрел субъект, отсортированные и без повторов; пустой список означает, что не смотрел никто. Факты о просмотрах неизвестны для сериалов (show_watch_rollup_unavailable), когда не подключён ни один медиасервер (no_media_server_connection), когда подключение ни разу не синхронизировалось начисто (signal_sync_never_succeeded), когда последняя чистая синхронизация старше 48 часов (signals_stale), когда ни у кого нет подтверждённой привязанной учётной записи медиасервера (no_linked_participants) и когда просмотр невозможно связать с пользователем Scryer (signal_identity_missing).",
  "settings.refMaintFactsLastWatchedAtValue":
    "Метка времени RFC3339 последнего просмотра; absent с причиной never_watched, если субъект никто не смотрел",
  "settings.refMaintFactsWatchedByAnyRequesterValue":
    "True, когда субъект смотрел хотя бы один запросивший; absent с причиной title_not_requested, если субъект никто не запрашивал, и unknown с причиной requester_not_linked, если у запросившего нет привязанной учётной записи медиасервера",
  "settings.refMaintFactsWatchedByAllRequestersValue":
    "True, когда субъект смотрели все запросившие; absent с причиной title_not_requested, если субъект никто не запрашивал, и unknown с причиной requester_not_linked, если у запросившего нет привязанной учётной записи медиасервера",
  "settings.refMaintFactsRequestLeaseStateValue":
    "none, когда для субъекта никогда не записывалось хранение по запросу; dormant, когда хранение ждёт первого импорта; active, пока хранение действует; expired, когда все сроки истекли. Снятое или заменённое хранение считается отозванным, а не истёкшим, поэтому оно не делает состояние expired. Истечение измеряется относительно времени оценки, поэтому хранение читается как expired в тот же момент, когда закрывается его окно.",
  "settings.refMaintFactsRequestLeaseExpiresAtValue":
    "Отметка времени RFC3339 самого позднего истечения среди действующих сроков; absent с причиной no_live_lease, когда субъект ничем не удерживается",
  "settings.refMaintFactsActiveRetentionClaimsValue":
    "Количество сроков хранения, всё ещё удерживающих субъект, включая ожидающие импорта",
  "settings.refMaintSectionFiles":
    "— По одной записи на файл в input.facts.files",
  "settings.refMaintFileSizeBytes": "Размер файла в байтах",
  "settings.refMaintFileQuality":
    "Определённое качество файла (например, 2160P, 1080P)",
  "settings.refMaintFileVideoCodec":
    "Видеокодек файла (например, H.265, H.264)",
  "settings.refMaintFileVideoWidth": "Ширина видео в пикселях",
  "settings.refMaintFileVideoHeight": "Высота видео в пикселях",
  "settings.refMaintFileAudioLanguages": "Языки аудиодорожек в файле",
  "settings.refMaintFileSubtitleLanguages": "Языки субтитров в файле",
  "settings.refMaintFileAddedAt": "Метка времени RFC3339 добавления файла",
  "settings.refMaintFactsSeriesMovies":
    "Фильмы, привязанные к таймлайну сериала, каждый со своими тегами. Только для сериалов: у фильма этот ключ отсутствует, а у сериала без привязанных фильмов он известен и пуст. add_tags и remove_tags никогда не пишут сюда — они действуют на сам тайтл.",
  "settings.refMaintFactsSeriesMoviesValue":
    "По одной записи на привязанный фильм; см. input.observations.series_movies.value[] ниже",
  "settings.refMaintSectionSeriesMovies":
    "— По одной записи на привязанный фильм в input.facts.series_movies",
  "settings.refMaintSectionObsSeriesMovies":
    "— По одной записи на привязанный фильм в input.observations.series_movies.value",
  "settings.refMaintSeriesMovieLinkId": "Идентификатор связи сериала и фильма",
  "settings.refMaintSeriesMovieName": "Название привязанного фильма",
  "settings.refMaintSeriesMovieYear":
    "Год выпуска привязанного фильма, если известен",
  "settings.refMaintSeriesMovieMonitored":
    "Включено ли получение для этого привязанного фильма",
  "settings.refMaintSeriesMovieHasFile":
    "Есть ли у этого привязанного фильма файл на диске",
  "settings.refMaintSeriesMovieTags":
    "Пользовательские теги этого привязанного фильма из того же реестра, что и теги тайтлов. Фильм сериала несёт свои теги и не наследует теги сериала.",
  "settings.refMaintTagActionsTitle": "Действия с тегами",
  "settings.refMaintTagActionsNote":
    "add_tags и remove_tags записывают пользовательские теги на совпавший тайтл. Каждая метка должна существовать в реестре тегов, который администратор ведёт в настройках, и хранится в нижнем регистре: правило, называющее удалённый тег, удерживается с причиной tag_not_defined вместо записи.",
  "settings.refMaintTagOscillationNote":
    "Два правила, записывающие противоположные изменения одной метки на одном тайтле, удерживают обоих кандидатов с причиной tag_patch_conflict, а не отдают победу тому, кто отработал последним. Правило, которое добавляет метку, и другое, которое снимает её на следующем прогоне, так не ловится: это ошибка автора, из-за которой тег переключается бесконечно, поэтому ограничьте их так, чтобы они не совпадали одновременно.",
  "settings.maintenanceTemplateGallery": "Готовые шаблоны",
  "settings.maintenanceTemplateGalleryDescription":
    "Загрузите готовый матчер в редактор. Ничего не сохраняется, пока вы не проверите правило и не нажмёте «Создать».",
  "settings.maintenanceTemplateApply": "Использовать шаблон",
  "settings.maintenanceTemplateFacetMovie": "Фильмы",
  "settings.maintenanceTemplateFacetShow": "Сериалы",
  "settings.maintenanceTemplateGraceBadge": "Отсрочка {{count}} дн.",
  "settings.maintenanceTemplateNoGraceBadge": "Без отсрочки",
  "settings.maintenanceTemplateDestructiveBadge": "Удаляет файлы",
  "settings.maintenanceTemplateNeedsProfileBadge": "Выберите целевой профиль",
  "settings.maintenanceTemplateDeadWantedTitle": "Мёртвые запрошенные записи",
  "settings.maintenanceTemplateDeadWantedDescription":
    "Отслеживаемые тайтлы, для которых так и не появился ни один файл. Факты — это обычные значения, поэтому тайтл, у которого Scryer не видит состояние отслеживания или наличия файла, удерживается, а не совпадает. После отсрочки в 30 дней область снимается с отслеживания, а всё, что уже есть на диске, остаётся нетронутым.",
  "settings.maintenanceTemplateLibraryAgingTitle": "Устаревание библиотеки",
  "settings.maintenanceTemplateLibraryAgingDescription":
    "Внимание: этот шаблон удаляет тайтл вместе с файлами. Всё, у чего есть файл, устаревает, если на нём нет тега «keep». Сначала идёт отсрочка в 180 дней, и прежде чем что-либо будет удалено, должны совпасть общие ограничители экземпляра и собственное вооружение правила.",
  "settings.maintenanceTemplateAddedLongAgoTitle":
    "Добавлено более 180 дней назад",
  "settings.maintenanceTemplateAddedLongAgoDescription":
    "Настоящая арифметика дат: матчер вычитает added_at тайтла из времени вычисления и совпадает после 180 дней. Область снимается с отслеживания, файлы остаются. Тайтл без значения added_at удерживается, а не совпадает.",
  "settings.maintenanceTemplateOversizedTitle": "Слишком большие релизы",
  "settings.maintenanceTemplateOversizedDescription":
    "Тайтлы, файлы которых в сумме занимают более 40 ГБ. Переводит тайтл на выбранный вами профиль качества и запускает поиск заново, только если профиль действительно изменился — выберите целевой профиль в редакторе до сохранения.",
  "settings.maintenanceTemplateFourKPurgeTitle": "Очистка 4K",
  "settings.maintenanceTemplateFourKPurgeDescription":
    "Любой тайтл, у которого есть файл 2160p. После отсрочки в 7 дней он переводится на выбранный вами профиль качества, и поиск запускается заново, если профиль изменился. Выберите целевой профиль в редакторе до сохранения.",
  "settings.maintenanceTemplateRequestedExpiryTitle":
    "Истечение срока запрошенного",
  "settings.maintenanceTemplateRequestedExpiryDescription":
    "Внимание: этот шаблон удаляет тайтл вместе с файлами. Тайтлы, пришедшие по запросу, устаревают через 120 дней, если на них нет тега «keep».",
  "settings.maintenanceTemplateDepartedRequesterTitle": "Ушедший заказчик",
  "settings.maintenanceTemplateDepartedRequesterDescription":
    "Внимание: этот шаблон удаляет тайтл вместе с файлами. Он совпадает с тайтлами, запрошенными одним указанным пользователем — замените в матчере заполнитель «departed-user» на нужное имя пользователя до сохранения, иначе правило не совпадёт ни с чем.",
  "settings.maintenanceTemplateSystemAddedTitle":
    "Очистка добавленного системой",
  "settings.maintenanceTemplateSystemAddedDescription":
    "Тайтлы, которые обнаружило сканирование, а не добавил человек, и у которых есть файл. Через 60 дней область снимается с отслеживания, а файлы остаются на месте.",
  "settings.maintenanceTemplateWatchedRequestedTitle":
    "Посмотрели все, кто запрашивал",
  "settings.maintenanceTemplateWatchedRequestedDescription":
    "Внимание: этот шаблон удаляет тайтл вместе с файлами. Он совпадает с запрошенными фильмами, которые посмотрели все запросившие их пользователи, после отсрочки в 30 дней. Только фильмы: Scryer записывает просмотры по фильмам и эпизодам, но не по сериалам, поэтому сериал удерживается, а не совпадает. Правило также удерживается, если у запросившего нет привязанной учётной записи медиасервера или последняя синхронизация сигналов старше 48 часов — неполная картина просмотров не является основанием для удаления.",
  "settings.maintenanceTemplateExpiredLeasesTitle":
    "Истёкшее хранение запросов",
  "settings.maintenanceTemplateExpiredLeasesDescription":
    "Внимание: этот шаблон удаляет тайтл вместе с файлами. Он совпадает с тайтлами, у которых истекли все сроки хранения по запросам, после отсрочки в 7 дней. Хранение, которое ещё не началось — тайтл не импортирован, — остаётся нетронутым, как и тайтл, у которого срока хранения никогда не было.",
  "settings.maintenanceTemplateNoProfileTitle": "Без профиля качества",
  "settings.maintenanceTemplateNoProfileDescription":
    "Тайтлы, которым не назначен профиль качества. Не выполняет никаких действий: этот шаблон нужен, чтобы собрать список и посмотреть на него, прежде чем решать, что с ним делать.",
  "settings.maintenanceTemplateNewestEpisodesTitle":
    "Оставить 3 новейших загруженных эпизода",
  "settings.maintenanceTemplateNewestEpisodesDescription":
    "Внимание: этот шаблон для серий отключает мониторинг и удаляет скачанные серии, которые по дате выхода старше трёх самых новых. Спецвыпуски, будущие серии, отсутствующие файлы и серии с неизвестной датой не участвуют в ранжировании и не подпадают под правило. Защищённые серии всё равно занимают места среди трёх самых новых.",
  "settings.maintenanceTemplateNewestSeasonsTitle":
    "Оставить 2 новейших загруженных сезона",
  "settings.maintenanceTemplateNewestSeasonsDescription":
    "Внимание: этот шаблон для сезонов отключает мониторинг и удаляет файлы сезонов, которые старше двух самых новых. Сезоны ранжируются по самой новой подходящей скачанной серии каждого сезона. Спецвыпуски, будущие серии, отсутствующие файлы и серии с неизвестной датой не участвуют в ранжировании; защищённые сезоны всё равно занимают места в этом списке.",
  "settings.maintenanceTemplateLeftEveryListTitle": "Выбыло из всех списков",
  "settings.maintenanceTemplateLeftEveryListDescription":
    "Тайтлы, из которых выбыли все списки, публичные и личные, когда каждое выбытие обработано и последнее произошло более 30 дней назад. Область снимается с отслеживания, а файлы остаются на месте.",
  "settings.maintenanceTemplateListAddedUnwatchedTitle":
    "Добавлено списком и ни разу не просмотрено",
  "settings.maintenanceTemplateListAddedUnwatchedDescription":
    "Добавляет тег needs-review к тайтлам, которые список добавил более 90 дней назад и которые никто не смотрел. Удерживается, пока картина просмотров неполная. Ничего не удаляется.",
  "settings.requestRules": "Правила запросов",
  "settings.noRequestRulesFound": "Правил запросов пока нет.",
  "settings.requestRuleLibraries": "Библиотеки",
  "settings.requestRuleLibrariesAll": "Все библиотеки",
  "settings.requestRuleLibrariesHelp":
    "Оставьте все флажки снятыми, чтобы правило применялось к запросам во всех библиотеках.",
  "settings.requestRuleRevision": "Ревизия",
  "settings.requestRuleDecisionCount": "Решений",
  "settings.requestRuleCopy": "Копировать правило",
  "settings.requestRuleCreate": "Создать правило",
  "settings.requestRuleUpdate": "Обновить правило",
  "settings.requestRuleCreateNew": "Новое правило запросов",
  "settings.requestRuleCreated":
    "Правило запросов создано. Оно отключено, пока вы его не включите.",
  "settings.requestRuleUpdated": "Правило запросов обновлено.",
  "settings.requestRuleDeleted": "Правило запросов {{name}} удалено.",
  "settings.requestRuleDeleting":
    "Удалить правило запросов {{name}}? Записанные им решения сохранятся.",
  "settings.requestRuleValidationRequired":
    "Правилу запросов нужны имя и сопоставитель.",
  "settings.requestRuleConfirmDiscardTitle":
    "Отменить несохранённые изменения?",
  "settings.requestRuleConfirmDiscardDescription":
    "Сопоставитель в редакторе не сохранён. Продолжение заменит его.",
  "settings.requestRuleRequesters": "Назовите людей в этом правиле",
  "settings.requestRuleRequestersHelp":
    "Отметьте учётные записи, о которых это правило, и примените их. Сопоставитель в редакторе будет переписан, чтобы вы прочитали его перед сохранением.",
  "settings.requestRuleRequestersApply": "Вписать эти имена в сопоставитель",
  "settings.requestRuleRequestersUnavailable":
    "Для списка учётных записей нужны права, которых у этой страницы нет, поэтому имена придётся вписать в сопоставитель вручную.",
  "settings.requestRuleRequestersNoTarget":
    "Этот сопоставитель никого не называет, поэтому выбранные учётные записи некуда вписать. Сначала добавьте набор requesters или сравнение по имени пользователя.",
  "settings.requestPreviewTitle": "Проверить правило на запросе",
  "settings.requestPreviewSubtitle":
    "Выполняет один сопоставитель на запросе, который вы опишете. Ничего не отправляется и не записывается.",
  "settings.requestPreviewSource": "Сопоставитель",
  "settings.requestPreviewSourceStored": "Сохранённое правило",
  "settings.requestPreviewSourceDraft": "Черновик в редакторе",
  "settings.requestPreviewRuleSelect": "Правило",
  "settings.requestPreviewRuleSelectPlaceholder": "Выберите правило",
  "settings.requestPreviewUser": "Запросивший",
  "settings.requestPreviewUserPlaceholder": "Выберите учётную запись",
  "settings.requestPreviewLibrary": "Библиотека",
  "settings.requestPreviewLibraryPlaceholder": "Выберите библиотеку",
  "settings.requestPreviewProfile": "Профиль качества",
  "settings.requestPreviewProfilePlaceholder": "Выберите профиль",
  "settings.requestPreviewMonitor": "Отслеживание",
  "settings.requestPreviewLease": "Хранить",
  "settings.requestPreviewLeaseDays": "Дней",
  "settings.requestPreviewMedia": "Медиа",
  "settings.requestPreviewMediaPlaceholder": "Найдите тайтл",
  "settings.requestPreviewMediaSelected": "Используется {{title}}.",
  "settings.requestPreviewRun": "Выполнить проверку",
  "settings.requestPreviewRunning": "Выполняется…",
  "settings.requestPreviewEmpty":
    "Опишите запрос и запустите проверку, чтобы увидеть, что скажет это правило.",
  "settings.requestPreviewMetadataPartial": "Метаданные неполные",
  "settings.requestPreviewShowInput": "Показать входные данные сопоставителя",
  "settings.requestPreviewHideInput": "Скрыть входные данные",
  "settings.requestPreviewInputUnavailable":
    "Не удалось прочитать входной документ.",
  "settings.requestPreviewUndefinedTags":
    "Определите их в «Настройки › Теги», прежде чем их можно будет применить.",
  "settings.requestDecisionsTitle": "Недавние решения",
  "settings.requestDecisionsOutcomeFilter": "Итог",
  "settings.requestDecisionsOutcomeAll": "Любой итог",
  "settings.requestDecisionsColTime": "Оценено",
  "settings.requestDecisionsColOutcome": "Итог",
  "settings.requestDecisionsColVotes": "Голоса",
  "settings.requestDecisionsColReasons": "Причины",
  "settings.requestDecisionsEmpty": "Решений пока не записано.",
  "settings.requestDecisionsPolicySaid": "Правила сказали {{outcome}}.",
  "settings.requestDecisionNoVotes": "Ни одно правило не проголосовало.",
  "settings.requestDecisionHeld": "удержано",
  "settings.requestTemplateGallery": "Готовые правила",
  "settings.requestTemplateGalleryDescription":
    "Разобранные примеры, которые можно загрузить в редактор. Загрузка заполняет форму; вы всё равно читаете правило, называете его, задаёте область и сохраняете.",
  "settings.requestTemplateGalleryTagNote":
    "Шаблону, который выдаёт тег, нужен этот тег в «Настройки › Теги»; неопределённые теги отбрасываются при решении по запросу.",
  "settings.requestTemplateApply": "Загрузить в редактор",
  "settings.requestTemplatePersonTargetedBadge": "Называет людей",
  "settings.requestTemplateNamedRequestersTitle":
    "Названные пользователи, семейный рейтинг",
  "settings.requestTemplateNamedRequestersDescription":
    "Одобряет запросы трёх названных людей, когда возрастной рейтинг семейный, и помечает самые семейные из них. Имена — заглушки: замените их через выбор учётных записей перед сохранением.",
  "settings.requestTemplateShortLeaseTitle":
    "Короткое хранение для одного человека",
  "settings.requestTemplateShortLeaseDescription":
    "Одобряет запросы одного названного человека, когда он просит хранить медиа не больше двух недель. Запрос «хранить вечно» не содержит числа дней, поэтому здесь не одобряется.",
  "settings.requestTemplateLowResolutionTitle": "Запросы с низким разрешением",
  "settings.requestTemplateLowResolutionDescription":
    "Одобряет запросы одного названного человека, когда выбранный им профиль качества ограничен 720p или ниже.",
  "settings.requestTemplateDenyAdultTitle": "Отклонять контент для взрослых",
  "settings.requestTemplateDenyAdultDescription":
    "Отклоняет любой запрос на медиа, помеченное в метаданных как контент для взрослых, и записывает причину. Никого не называет, поэтому применяется ко всем.",
  "settings.requestTemplateMonthlyQuotaTitle": "Месячная квота одобрений",
  "settings.requestTemplateMonthlyQuotaDescription":
    "Отправляет запрос на рассмотрение, когда у запросившего уже пять одобрений за последние тридцать дней. Никогда не отклоняет — просит человека посмотреть.",
  "settings.requestTemplateHoldApprovalListsTitle":
    "Удерживать запросы из списков участников с политикой «Одобрение»",
  "settings.requestTemplateHoldApprovalListsDescription":
    "Отправляет запрос из списка на рассмотрение, когда у владельца списка политика списков «Одобрение». Ручные запросы не затрагиваются.",
  "settings.requestTemplateDenyPersonalListAbove1080pTitle":
    "Отклонять запросы из личных списков выше 1080p",
  "settings.requestTemplateDenyPersonalListAbove1080pDescription":
    "Отклоняет запрос из личного списка участника, когда его профиль качества выше 1080p. Публичные списки и ручные запросы не затрагиваются.",
  "settings.refReqTitle": "Справочник входных данных запросов",
  "settings.refReqSubtitle":
    "Все значения, которые может прочитать сопоставитель запроса, и что каждое из них означает.",
  "settings.refReqIntro":
    "Правило запроса читает один документ, описывающий запрос, того, кто его сделал, целевую библиотеку и то, что Scryer знает о медиа. Факты — это обычные значения. Факт, который Scryer не смог наблюдать, просто отсутствует, и правило, читающее отсутствующий факт, удерживает запрос для человека, а не гадает — эту защиту вы никогда не пишете сами.",
  "settings.refReqOutputTitle": "Что определяет правило запроса",
  "settings.refReqOutputIntro":
    "Должно быть определено хотя бы одно из них, иначе правило никогда не сможет проголосовать и Scryer откажется его сохранить.",
  "settings.refReqOutputApprove":
    "approve — запрос можно удовлетворить без участия человека.",
  "settings.refReqOutputDeny":
    "deny — запрос отклоняется. Отказ перевешивает любой другой голос.",
  "settings.refReqOutputManual":
    "manual — запрос уходит к человеку, что бы ни решили остальные.",
  "settings.refReqOutputTags":
    'tags contains "…" — метки для медиа, применяются только при одобрении.',
  "settings.refReqOutputReasons":
    'reasons contains "…" — устойчивые коды, которые видит запросивший.',
  "settings.refReqOutputNoPackage":
    "Не пишите строки package и import. Scryer добавляет их сам и убирает обратно, когда передаёт сопоставитель в этот редактор.",
  "search.keepFor": "Хранить",
  "search.keepForForever": "Вечно",
  "search.keepForDays": "{{count}} дней",
  "search.keepForCustom": "Указать число дней…",
  "search.keepForCustomLabel": "Дней хранения",
  "search.keepForHelp":
    "Сколько Scryer будет хранить это медиа для вас. Когда срок истечёт, правило обслуживания может его удалить; вечное хранение не истекает.",
  "requests.preflightAutoApprove": "Это будет одобрено автоматически",
  "requests.preflightManualReview": "Это требует одобрения",
  "requests.preflightDeny": "Это было бы отклонено: {{reasons}}",
  "requests.preflightTags": "Будут добавлены метки: {{tags}}.",
  "requests.preflightReason": "Причина: {{reason}}.",
  "requests.preflightMetadataPartial":
    "Scryer не смог прочитать всё об этом тайтле, поэтому решение примет человек.",
  "requests.leaseForever": "Хранится вечно",
  "requests.leaseRequested": "Запрошено {{count}} дней",
  "requests.leaseDormant": "{{count}} дней с первого импорта",
  "requests.leaseActive": "Хранится до {{date}}",
  "requests.leaseExpired": "Срок хранения истёк",
  "requests.leaseReleased": "Хранение снято",
  "requests.decisionTitle": "Как это было решено",
  "requests.decisionPolicySaid":
    "Правила сказали {{outcome}}; запрос решило что-то другое.",
  "requests.decisionVotes": "Голоса правил",
  "requests.decisionNothingToShow":
    "Ни одно правило ничего не сказало об этом запросе.",
  "requests.decisionOutcomeAutoApprove": "Одобрено правилом",
  "requests.decisionOutcomeManualReview": "Отправлено на рассмотрение",
  "requests.decisionOutcomeDeny": "Отклонено правилом",
  "requests.denyReason": "Отклонено, потому что: {{reasons}}",
  "requests.policyTagsLabel": "Будут метки",
  "requests.voteApprove": "одобрить",
  "requests.voteDeny": "отклонить",
  "requests.voteManual": "рассмотреть",
  "requests.voteAbstain": "воздержался",
  "requests.voteNone": "нет голоса",
  "requests.fallbackRuleManual":
    "Правило попросило человека посмотреть на это.",
  "requests.fallbackHeld":
    "Правило не смогло решить, потому что что-то из читаемого им неизвестно.",
  "requests.fallbackError":
    "Правило не выполнилось, поэтому запрос ушёл к человеку.",
  "requests.fallbackNoRuleMatched":
    "Ни одно правило не подошло к этому запросу.",
  "requests.approvedLease": "Хранить",
  "requests.approvedLeaseKeepForever": "Вечно, как и просили",
  "requests.approvedLeaseKeepRequested": "{{count}} дней, как и просили",
  "requests.approvedTags": "Метки",
  "requests.approvedTagsHelp":
    "Применяются к тайтлу при одобрении. Заполнены тем, что выдали правила; добавляйте и убирайте любой тег, определённый администратором в «Настройки › Теги».",
  "requests.approvedTagsNone": "Теги не будут применены.",
  "requests.claimsShow": "Хранение",
  "requests.claimsHide": "Скрыть хранение",
  "requests.claimsEmpty": "Этот тайтл ничем не удерживается.",
  "requests.claimExpiresAt": "Удерживается до {{date}}",
  "requests.claimDurationDays": "{{count}} дней, начиная с первого импорта",
  "requests.claimReleasedReason": "Снято: {{reason}}",
  "requests.claimStateDormant": "Ожидает импорта",
  "requests.claimStateActive": "Удерживает",
  "requests.claimStateExpired": "Истекло",
  "requests.claimStateReleased": "Снято",
  "requests.claimStateConverted": "Заменено",
  "requests.claimExtend": "Продлить",
  "requests.claimExtendTitle": "Продлить это удержание",
  "requests.claimExtendUntil": "Удерживать до",
  "requests.claimMakePermanent": "Хранить постоянно",
  "requests.claimRelease": "Снять",
  "requests.claimReleaseTitle": "Снять это удержание",
  "requests.claimReleaseReason": "Причина",
  "requests.claimReleaseReasonPlaceholder": "Почему это удержание снимается",
  "requests.claimReleaseReasonHelp":
    "Причина обязательна; она записывается вместе с удержанием.",
  "status.requestApprovedClaimFailed":
    "{{name}} одобрено, но ничем не удерживается: {{error}}",
  "status.claimExtended": "Удержание продлено.",
  "status.claimMadePermanent": "Тайтл теперь удерживается постоянно.",
  "status.claimReleased": "Удержание снято.",
  "settings.refReqSectionInput": "— Запрос, по которому принимается решение",
  "settings.refReqInputSchemaVersion":
    "Версия формы этого документа; меняется только при добавлении полей",
  "settings.refReqInputEvaluationTime":
    "Отметка времени RFC3339, когда запрос был оценён",
  "settings.refReqSectionNow":
    "— Часы на момент оценки, для правил, зависящих от времени поступления запроса",
  "settings.refReqNowWeekday":
    "День недели по UTC, строчными буквами и по-английски (monday … sunday)",
  "settings.refReqNowHourUtc": "Час суток по UTC, от 0 до 23",
  "settings.refReqSectionRequester":
    "— Человек, сделавший запрос. Сопоставителю, читающему любое из этих полей, для сохранения нужны права управления разрешениями",
  "settings.refReqRequesterUserId": "Идентификатор запросившей учётной записи",
  "settings.refReqRequesterUsername":
    "Имя пользователя запросившей учётной записи",
  "settings.refReqRequesterAccountKind":
    "Как учётная запись входит в систему (например, локально или через связанного поставщика)",
  "settings.refReqRequesterAppPermissions":
    "Общие права приложения, которыми обладает учётная запись",
  "settings.refReqRequesterLibraryPermissions":
    "Права учётной записи в библиотеке, из которой делается запрос",
  "settings.refReqRequesterLinkedProviders":
    "Медиасерверы и поставщики учётных данных, привязанные к учётной записи",
  "settings.refReqRequesterCreatedAt":
    "Когда создана учётная запись; отсутствует, потому что Scryer пока это не записывает",
  "settings.refReqSectionLibrary": "— Библиотека, в которую направлен запрос",
  "settings.refReqLibraryId": "Идентификатор библиотеки",
  "settings.refReqLibraryName": "Название библиотеки",
  "settings.refReqLibraryFacet":
    "Вид медиа в библиотеке (MOVIE, SERIES или ANIME)",
  "settings.refReqLibraryIsDefault":
    "Истина, когда это библиотека по умолчанию для своего вида",
  "settings.refReqSectionRequest": "— Что было запрошено",
  "settings.refReqRequestOrigin":
    "Откуда пришёл запрос (например, из диалога поиска)",
  "settings.refReqRequestTitle": "Название на момент запроса",
  "settings.refReqRequestYear": "Год выпуска, если поставщик его знал",
  "settings.refReqRequestExternalIds":
    "Идентификаторы поставщиков; см. input.request.external_ids ниже",
  "settings.refReqRequestQualityProfileId":
    "Идентификатор профиля качества, выбранного запросившим",
  "settings.refReqRequestQualityProfileName": "Название этого профиля",
  "settings.refReqRequestMonitorType":
    "Отслеживание, которое запросил пользователь (например, ALL_EPISODES)",
  "settings.refReqRequestMonitorSelectionSeasonCount":
    "Сколько сезонов названо в расширенном выборе отслеживания",
  "settings.refReqRequestLeaseForever":
    "Истина, когда запросивший попросил хранить медиа вечно",
  "settings.refReqRequestLeaseDays":
    "Сколько дней запросивший просил хранить медиа; отсутствует, когда он попросил хранить вечно",
  "settings.refReqSectionExternalIds":
    "— Идентификаторы поставщиков в input.request.external_ids",
  "settings.refReqExternalIdsTmdb":
    "Идентификатор TMDB, если он был у поставщика",
  "settings.refReqExternalIdsImdb":
    "Идентификатор IMDb, если он был у поставщика",
  "settings.refReqExternalIdsTvdb":
    "Идентификатор TVDB, если он был у поставщика",
  "settings.refReqSectionFacts":
    "— Что Scryer знает о медиа и об истории этого пользователя. Факт, который Scryer не смог наблюдать, отсутствует, и чтение отсутствующего факта удерживает запрос для человека",
  "settings.refReqFactsAgeRating":
    "Минимальный возраст, подразумеваемый возрастным рейтингом",
  "settings.refReqFactsCertifications":
    "Все возрастные рейтинги из метаданных; см. input.facts.certifications[] ниже",
  "settings.refReqFactsCertificationLabel":
    "Американский возрастной рейтинг (например, G, PG, PG-13, R)",
  "settings.refReqFactsCertificationRank":
    "Этот рейтинг числом, от меньшего к большему, чтобы правила могли сравнивать, а не перечислять метки",
  "settings.refReqFactsCommonsenseRecommended":
    "Рекомендует ли Common Sense Media тайтл для его возрастной категории",
  "settings.refReqFactsGenres": "Названия жанров тайтла",
  "settings.refReqFactsCanonicalTagKeys":
    "Канонические ключи меток Scryer для тайтла",
  "settings.refReqFactsThemes": "Названия тем тайтла",
  "settings.refReqFactsIsAdult":
    "Помечен ли тайтл в метаданных как контент для взрослых",
  "settings.refReqFactsRating": "Сводная оценка из метаданных",
  "settings.refReqFactsRatingsBySource":
    "Те же оценки по источникам, которые их выставили",
  "settings.refReqFactsTmdbVoteAverage": "Средняя оценка TMDB",
  "settings.refReqFactsTmdbVoteCount":
    "Из скольких голосов складывается эта средняя оценка",
  "settings.refReqFactsPopularity": "Показатель популярности у поставщика",
  "settings.refReqFactsRuntimeMinutes": "Длительность в минутах",
  "settings.refReqFactsOriginalLanguage": "Код языка оригинала",
  "settings.refReqFactsCountry": "Страна производства",
  "settings.refReqFactsNetwork": "Телесеть, которая его показывала",
  "settings.refReqFactsStudio": "Студия, которая его сняла",
  "settings.refReqFactsContentStatus":
    "Статус контента у поставщика (например, Continuing, Ended)",
  "settings.refReqFactsReleaseDate": "Дата выхода, для фильмов",
  "settings.refReqFactsFirstAired": "Дата первого показа, для сериалов",
  "settings.refReqFactsReleaseAgeDays":
    "Сколько дней назад состоялся выход или первый показ",
  "settings.refReqFactsAwardCount": "Сколько наград записано в метаданных",
  "settings.refReqFactsQualityProfileTiers":
    "Названия уровней в профиле качества, выбранном запросившим",
  "settings.refReqFactsQualityProfileMaxResolution":
    "Наибольшее вертикальное разрешение, разрешённое профилем, в пикселях",
  "settings.refReqFactsQualityProfileAllowsUpgrades":
    "Продолжает ли профиль улучшать релиз после первой загрузки",
  "settings.refReqFactsExistsInLibraryIds":
    "Библиотеки, в которых этот тайтл уже есть",
  "settings.refReqFactsPreviousRequestCount":
    "Сколько раз этот пользователь уже запрашивал этот тайтл",
  "settings.refReqFactsPreviouslyDenied":
    "Был ли один из тех прежних запросов отклонён",
  "settings.refReqFactsPreviouslyApproved":
    "Был ли один из тех прежних запросов одобрен",
  "settings.refReqFactsPendingRequestCount":
    "Сколько запросов этого пользователя всё ещё ждут решения",
  "settings.refReqFactsApprovedLast30d":
    "Сколько запросов этого пользователя одобрено за последние тридцать дней",
  "settings.refReqFactsDeniedLast30d":
    "Сколько отклонено за последние тридцать дней",
  "settings.refReqFactsTotalApproved":
    "Сколько запросов этого пользователя было одобрено за всё время",
  "settings.refReqFactsActiveLeaseCount":
    "На сколько тайтлов у этого пользователя сейчас действует хранение",
  "settings.refReqFactsDaysSinceLastRequest":
    "Сколько дней прошло с прошлого запроса этого пользователя",
  "settings.refReqFactsLibraryTitleCount":
    "Сколько тайтлов в целевой библиотеке",
  "settings.refReqSectionCertifications":
    "— По одной записи на каждый рейтинг в input.facts.certifications",
  "settings.refReqCertCountry": "Страна, в которой выдан рейтинг",
  "settings.refReqCertValue": "Сам рейтинг (например, PG-13)",
  "settings.refReqCertSource": "Откуда получен рейтинг",
  "settings.refReqSectionObservations":
    "— Те же факты в виде полных конвертов, для правил, которым нужно отличать «отсутствует» от «неизвестно» или прочитать причину. Чтение факта здесь отключает для него автоматическое удержание: вы берёте на себя ответственность за трёхзначный ответ.",
  "settings.refReqObsStatus":
    "known, absent или unknown. absent означает, что источник ответил и значения нет; unknown — что Scryer не смог выяснить. Проверяйте это перед чтением value: неизвестный факт никогда не превращается в false, 0 или пустую строку.",
  "settings.refReqObsObservedAt":
    "Когда значение было получено, если источник записал время",
  "settings.refReqObsReason":
    "Устойчивый машинный код, объясняющий, почему факт неизвестен или почему у отсутствующего факта нет значения",
  "settings.refReqSectionObsCertifications":
    "— По одной записи на каждый рейтинг в input.observations.certifications.value",
  "settings.refReqFactsAgeRatingValue":
    "Минимальный возраст по рейтингу; absent с причиной no_certification, когда рейтинга в метаданных нет",
  "settings.refReqFactsCertificationsValue":
    "По одной записи на рейтинг; см. input.observations.certifications.value[] ниже",
  "settings.refReqFactsCertificationLabelValue":
    "Американский возрастной рейтинг; absent, когда американского рейтинга в метаданных нет",
  "settings.refReqFactsCertificationRankValue":
    "Этот рейтинг числом, от меньшего к большему; absent, когда Scryer не ранжирует такую метку",
  "settings.refReqFactsCommonsenseRecommendedValue":
    "Истина, когда Common Sense Media рекомендует тайтл для его возрастной категории",
  "settings.refReqFactsGenresValue":
    "Названия жанров без повторов; пустой список, когда метаданные их не называют",
  "settings.refReqFactsCanonicalTagKeysValue":
    "Канонические ключи меток без повторов",
  "settings.refReqFactsThemesValue": "Названия тем без повторов",
  "settings.refReqFactsIsAdultValue":
    "Истина, когда метаданные помечают тайтл как контент для взрослых",
  "settings.refReqFactsRatingValue": "Сводная оценка числом",
  "settings.refReqFactsRatingsBySourceValue":
    "Объект с ключами по названию источника, значение — оценка этого источника",
  "settings.refReqFactsTmdbVoteAverageValue": "Средняя оценка TMDB числом",
  "settings.refReqFactsTmdbVoteCountValue":
    "Из скольких голосов складывается эта средняя оценка",
  "settings.refReqFactsPopularityValue": "Показатель популярности числом",
  "settings.refReqFactsRuntimeMinutesValue": "Длительность в целых минутах",
  "settings.refReqFactsOriginalLanguageValue": "Язык оригинала в виде кода",
  "settings.refReqFactsCountryValue": "Страна производства",
  "settings.refReqFactsNetworkValue": "Название телесети",
  "settings.refReqFactsStudioValue": "Название студии",
  "settings.refReqFactsContentStatusValue":
    "Строка статуса контента у поставщика",
  "settings.refReqFactsReleaseDateValue": "Дата выхода в формате RFC3339",
  "settings.refReqFactsFirstAiredValue":
    "Дата первого показа в формате RFC3339",
  "settings.refReqFactsReleaseAgeDaysValue":
    "Целых дней с выхода или первого показа",
  "settings.refReqFactsAwardCountValue":
    "Число записанных наград; подтверждённый ноль, когда наград нет",
  "settings.refReqFactsQualityProfileTiersValue":
    "Названия уровней выбранного профиля в порядке профиля",
  "settings.refReqFactsQualityProfileMaxResolutionValue":
    "Наибольшее вертикальное разрешение профиля в пикселях",
  "settings.refReqFactsQualityProfileAllowsUpgradesValue":
    "Истина, когда профиль продолжает улучшать релиз после первой загрузки",
  "settings.refReqFactsExistsInLibraryIdsValue":
    "Идентификаторы библиотек, в которых тайтл уже есть; пустой список, когда таких нет",
  "settings.refReqFactsPreviousRequestCountValue":
    "Число прежних запросов этого пользователя на этот тайтл",
  "settings.refReqFactsPreviouslyDeniedValue":
    "Истина, когда один из тех прежних запросов был отклонён",
  "settings.refReqFactsPreviouslyApprovedValue":
    "Истина, когда один из тех прежних запросов был одобрен",
  "settings.refReqFactsPendingRequestCountValue":
    "Число запросов этого пользователя, всё ещё ожидающих решения",
  "settings.refReqFactsApprovedLast30dValue":
    "Число одобренных за последние тридцать дней, отсчитывая от времени оценки",
  "settings.refReqFactsDeniedLast30dValue":
    "Число отклонённых за последние тридцать дней, отсчитывая от времени оценки",
  "settings.refReqFactsTotalApprovedValue":
    "Число когда-либо одобренных запросов этого пользователя",
  "settings.refReqFactsActiveLeaseCountValue":
    "Число тайтлов, на которые у пользователя сейчас есть хранение, включая ожидающие импорта",
  "settings.refReqFactsDaysSinceLastRequestValue":
    "Целых дней с прошлого запроса пользователя; absent с причиной no_previous_request, когда это его первый запрос",
  "settings.refReqFactsLibraryTitleCountValue":
    "Число тайтлов в целевой библиотеке",
  "move.dialogTitle": "Переместить тайтлы",
  "move.dialogDescription":
    "Проверьте план для {{count}} выбранных тайтлов, прежде чем что-либо перемещать.",
  "move.destinationLibrary": "Целевая библиотека",
  "move.destinationRoot": "Целевой корневой каталог",
  "move.destinationRootPlaceholder": "Выберите корневой каталог",
  "move.actionHeading": "Переместить в…",
  "move.actionDescription":
    "Переместить файлы этого тайтла в другой корневой каталог или в другую библиотеку.",
  "move.actionButton": "Переместить…",
  "move.wizardKindDescription": "Выберите, куда переместить этот тайтл.",
  "move.kindRootHeading": "Переместить в другой корневой каталог",
  "move.kindRootHelp":
    "Тайтл останется в этой библиотеке, а его файлы переедут в другой её корневой каталог.",
  "move.kindLibraryHeading": "Переместить в другую библиотеку",
  "move.kindLibraryHelp":
    "Перенести тайтл в другую библиотеку, в один из её корневых каталогов.",
  "move.noOtherRoots":
    "В этой библиотеке нет другого корневого каталога для перемещения. Добавьте его в библиотеку или переместите тайтл в другую библиотеку.",
  "move.back": "Назад",
  "move.next": "Далее",
  "move.destinationRootHelp":
    "Смена назначения открывает процесс перемещения вместо сохранения тайтла.",
  "move.destinationNoSelection": "Тайтлы не выбраны.",
  "move.destinationMixedSourceLibraries":
    "Выбор охватывает несколько библиотек; перемещайте по одной библиотеке за раз.",
  "move.destinationCrossLibraryNotice":
    "Эти тайтлы покинут текущую библиотеку и станут частью библиотеки {{library}}.",
  "move.destinationLibraryInMoveWorkflow":
    "Выберите другую библиотеку в мастере перемещения, после выбора корневой папки назначения.",
  "move.transferIntoLibrary": "Переносится в библиотеку {{library}}.",
  "move.sameNameWarning":
    "В библиотеке {{library}} уже есть тайтл с названием «{{title}}». Scryer никогда не объединяет по названию, поэтому после переноса останутся два тайтла с этим названием.",
  "move.ambiguousDestinationHelp":
    "Несколько тайтлов в библиотеке назначения имеют ту же идентификацию, что и этот, и Scryer не будет угадывать между ними. Сначала разберитесь с идентификацией в библиотеке назначения, затем переместите этот тайтл снова.",
  "move.ambiguousCandidateShared": "Общее: {{identities}}",
  "move.mergeStatement":
    "Объединяется с «{{title}}». Два тайтла становятся одним: идентификация этого тайтла поглощается, а настройки «{{title}}» имеют приоритет.",
  "move.mergeSummaryHeading": "Что делает это объединение",
  "move.mergeNoDetails": "Это объединение больше ничего не переносит.",
  "move.mergeBlockedHeading":
    "Объединение невозможно: эти записи нельзя перенести.",
  "move.mergeCarried":
    "Вместе с ним переходят медиафайлы ({{files}}) и записи истории ({{history}}).",
  "move.mergeDropped":
    "{{count}} прочая(-ие) запись(-и) объединяемого названия не переносятся и удаляются вместе с ним.",
  "move.mergeRoleChangesHeading":
    "Файлов, которые станут дополнительными копиями: {{demotions}}",
  "move.mergeRoleChangesHeadingPlain": "Роли файлов",
  "move.mergeRoleGroup.DESTINATION_PRIMARY_RETAINED":
    "У «{{title}}» уже есть основной файл для этих серий. Поступающие файлы сохраняются как дополнительные копии; ничего из имеющегося не заменяется.",
  "move.mergeRoleGroup.DESTINATION_PRIMARY_RETAINED_TITLE":
    "У «{{title}}» уже есть основной файл. Поступающий файл сохраняется как дополнительная копия; ничего из имеющегося не заменяется.",
  "move.mergeRoleGroup.SOURCE_PRIMARY_ALREADY_CLAIMED":
    "Другой файл из этого перемещения уже является основным для этих серий, поэтому эти сохраняются как дополнительные копии.",
  "move.mergeRoleGroup.SOURCE_PRIMARY_ALREADY_CLAIMED_TITLE":
    "Другой файл из этого перемещения уже является основным, поэтому этот сохраняется как дополнительная копия.",
  "move.mergeRoleGroup.COLLAPSED_SOURCE_EPISODES":
    "Эти файлы числятся здесь под двумя сериями, которые в месте назначения являются одной. Там каждый из них числится один раз.",
  "move.mergeRoleUnnamedFile": "файл {{id}}",
  "move.operationTransferNote": "Эти тайтлы перемещаются в другую библиотеку.",
  "move.modeHeading": "Как файлы должны туда попасть?",
  "move.mode.MOVE_WITH_SCRYER": "Переместить через Scryer",
  "move.mode.FILES_ALREADY_THERE": "Файлы уже на месте",
  "move.modeHelp.MOVE_WITH_SCRYER":
    "Scryer перемещает или копирует файлы и проверяет каждый до изменения каталога.",
  "move.modeHelp.FILES_ALREADY_THERE":
    "Scryer сканирует назначение и принимает файлы, которые вы перенесли сами.",
  "move.catalogOnlyFastPath":
    "У этих тайтлов нет файлов на диске, поэтому это переназначение только в каталоге: ничего не перемещается и режим выбирать не нужно.",
  "move.selectDestinationPrompt":
    "Выберите целевой корневой каталог, чтобы увидеть план.",
  "move.previewLoading": "Строим план…",
  "move.previewFailed": "Не удалось предпросмотреть перемещение.",
  "move.planChanged":
    "План изменился с момента предпросмотра. Проверьте новый план перед подтверждением.",
  "move.rePreview": "Предпросмотреть снова",
  "move.summaryTitles": "Тайтлы",
  "move.summaryFiles": "Файлы",
  "move.summarySize": "Общий размер",
  "move.sampledItems": "выборка",
  "move.blockedHeading": "{{count}} тайтлов пока нельзя переместить",
  "move.blockedHelp":
    "Снимите с них выбор, чтобы переместить остальные, или дождитесь завершения их работы.",
  "move.deselect": "Снять выбор",
  "move.deselectedCount": "{{count}} тайтлов исключено из этого перемещения.",
  "move.restoreSelection": "Восстановить выбор",
  "move.titleFileSummary": "{{files}} файлов · {{size}}",
  "move.freeSpaceRequired":
    "Требуется {{required}} в назначении; свободно {{available}}.",
  "move.freeSpaceUnknown": "неизвестно",
  "move.freeSpaceInsufficient": "В назначении недостаточно свободного места.",
  "move.startRefusedNoSpace":
    "В назначении недостаточно свободного места для этого перемещения. Освободите место или переместите меньше тайтлов и повторите попытку.",
  "move.freeSpaceNotProbed": "Не удалось измерить целевой том.",
  "move.freeSpaceRecycleOtherVolume":
    "Корзина находится на другом томе и требует {{required}}.",
  "move.verificationStatementFull":
    "Все {{files}} файлов ({{bytes}}) будут полностью перечитаны и сверены.",
  "move.verificationStatementQuick":
    "Все {{files}} файлов ({{bytes}}) будут проверены по началу, концу и размеру.",
  "move.verificationNotApplicable": "Файлы не перемещаются, проверять нечего.",
  "move.verificationStampFull": "проверено (полностью)",
  "move.verificationStampQuick": "проверено (быстро)",
  "move.verificationFallbackCount":
    "{{count}} файлов перешли на быструю проверку",
  "move.typedConfirmationPrompt": "Введите фразу для подтверждения",
  "move.confirm": "Начать перемещение",
  "move.startFailed": "Не удалось начать перемещение.",
  "move.operationLoading": "Загрузка операции…",
  "move.operationLoadFailed": "Не удалось загрузить операцию.",
  "move.operationMissing": "Такой операции больше нет.",
  "move.operationDismiss": "Закрыть",
  "move.operationProgress": "Тайтлы {{titles}} · Файлы {{files}} · {{bytes}}",
  "move.counterMerges": "Объединено",
  "move.counterDedups": "Дедуплицировано",
  "move.counterRenames": "Переименовано",
  "move.counterNoOps": "Без изменений",
  "move.counterUnresolved": "Не решено",
  "move.counterBlocked": "Заблокировано",
  "move.checkpointsHeading": "Тайтлы",
  "move.checkpointsPending":
    "Тайтлы появляются здесь по мере входа в операцию.",
  "move.checkpointClass": "Классифицировано как",
  "move.checkpointFrom": "Откуда",
  "move.checkpointTo": "Куда",
  "move.checkpointMergedInto": "Объединён с",
  "move.checkpointVerified": "Проверено",
  "move.checkpointVerifiedValue": "{{files}} файлов · {{bytes}}",
  "move.checkpointDetail": "Подробности",
  "move.assetsHeading": "Переименованные и дедуплицированные файлы",
  "move.assetRenamedAs": "{{from}} → {{to}}",
  "move.assetDeduplicatedAgainst":
    "{{from}} дедуплицирован по отношению к {{to}}",
  "move.assetUnknownCounterpart": "файл без имени",
  "move.assetProvenance": "(из {{library}})",
  "move.assetStateDone": "Выполнено",
  "move.assetStatePlanned": "Запланировано",
  "move.cancelAction": "Отменить операцию",
  "move.cancelRequested":
    "Отмена запрошена; операция остановится на следующем тайтле.",
  "move.cancelNotPossible": "Эта операция уже завершена.",
  "move.cancelFailed": "Не удалось запросить отмену.",
  "move.cancelDraining": "Остановка на следующем тайтле",
  "move.resumeAction": "Возобновить",
  "move.resumeRequested":
    "Возобновление с последней проверенной контрольной точки.",
  "move.resumeNotPossible": "Эту операцию нельзя возобновить.",
  "move.resumeFailed": "Не удалось возобновить операцию.",
  "move.class.ROOT_MOVE": "Перемещение в целевой корневой каталог",
  "move.class.CROSS_LIBRARY_TRANSFER": "Перенос в другую библиотеку",
  "move.class.CATALOG_ONLY": "Только каталог (файлов на диске нет)",
  "move.class.NO_OP": "Уже в назначении",
  "move.class.NEEDS_RESOLUTION": "Требуется решение",
  "move.class.INCOMPATIBLE": "Туда переместить нельзя",
  "move.planKind.MOVE": "Перемещения",
  "move.planKind.RENAME": "Переименования",
  "move.planKind.MERGE": "Слияния",
  "move.planKind.DEDUP": "Возможно слияние с идентичными файлами",
  "move.planKind.CATALOG_CHANGE": "Изменения каталога",
  "move.planKind.ROLE_CHANGE": "Изменения роли",
  "move.planKind.NO_OP": "Без изменений",
  "move.planKind.BLOCKED": "Заблокировано",
  "move.planKind.UNMANAGED_CONTENT": "Неуправляемое содержимое",
  "move.planKind.WARNING": "Предупреждения",
  "move.operationState.QUEUED": "В очереди",
  "move.operationState.PREPARING": "Подготовка",
  "move.operationState.MOVING": "Перемещение",
  "move.operationState.VERIFYING": "Проверка",
  "move.operationState.RECONCILING": "Сверка",
  "move.operationState.CLEANING_UP": "Очистка",
  "move.operationState.COMPLETED": "Завершено",
  "move.operationState.COMPLETED_WITH_WARNINGS": "Завершено с предупреждениями",
  "move.operationState.CANCELED": "Отменено",
  "move.operationState.FAILED": "Не удалось",
  "move.checkpointState.PENDING": "Ожидание",
  "move.checkpointState.MOVING": "Перемещение",
  "move.checkpointState.VERIFYING": "Проверка",
  "move.checkpointState.RECONCILING": "Сверка",
  "move.checkpointState.CLEANING_UP": "Очистка",
  "move.checkpointState.COMPLETED": "Завершено",
  "move.checkpointState.COMPLETED_WITH_WARNINGS":
    "Завершено с предупреждениями",
  "move.checkpointState.SKIPPED": "Пропущено",
  "move.checkpointState.BLOCKED": "Заблокировано",
  "move.checkpointState.FAILED": "Не удалось",
  "move.operationType.FOLDER_REASSIGNMENT": "Переназначение папки",
  "move.operationType.ROOT_MOVE": "Перемещение корневого каталога",
  "move.operationType.ROOT_CHANGE": "Смена корневого каталога",
  "move.operationType.ROOT_CONSOLIDATION": "Объединение корневых каталогов",
  "move.operationType.CROSS_LIBRARY_TRANSFER": "Перенос между библиотеками",
  "move.toastPreparing": "Подготовка…",
  "move.toastTransferred": "Передано",
  "move.toastTitlesProgress": "{{current}} / {{total}} тайтлов",
  "move.toastDoneSubtitle": "Перемещение завершено · {{count}} тайтлов",
  "move.toastWarningsSubtitle":
    "Завершено с предупреждениями · {{count}} тайтлов",
  "move.toastWarningsChip":
    "Некоторые тайтлы требуют проверки в разделе «Активность».",
  "move.toastFailedSubtitle": "Перемещение не удалось",
  "move.toastCanceledSubtitle": "Перемещение отменено",
  "move.startedHeading":
    "Перемещение началось. Следите за ним в разделе «Активность».",
  "move.viewInActivity": "Открыть в разделе «Активность»",
  "rootChange.rowAction": "Изменить корень",
  "rootChange.unavailableWhileUnsaved":
    "Сначала сохраните или отмените несохранённые изменения корней.",
  "rootChange.dialogTitle": "Изменить корень",
  "rootChange.dialogDescription":
    "Всё, что лежит в {path}, будет перемещено. Операция над всем корнем не может оставить ни одного тайтла.",
  "rootChange.destinationHeading": "Куда переезжает этот корень",
  "rootChange.destinationNewPath": "Новый путь",
  "rootChange.destinationNewPathHelp":
    "Корень сохраняет свою идентичность и привязки тайтлов; меняется только путь.",
  "rootChange.destinationExistingRoot": "Другой корень этой библиотеки",
  "rootChange.destinationExistingRootHelp":
    "Этот корень будет влит в выбранный, а его настройка выведена из обращения.",
  "rootChange.destinationNoOtherRoots":
    "В этой библиотеке нет другого корня, в который можно влить.",
  "rootChange.destinationPathPlaceholder": "/mnt/new-disk/movies",
  "rootChange.planChanged":
    "План изменился после предпросмотра. Постройте предпросмотр заново перед подтверждением.",
  "rootChange.previewFailed":
    "Не удалось построить предпросмотр этого изменения корня.",
  "rootChange.previewing": "Читаем весь корень.",
  "rootChange.preview": "Предпросмотр",
  "rootChange.confirm": "Запустить",
  "rootChange.startFailed": "Не удалось запустить это изменение корня.",
  "rootChange.startedHeading":
    "Изменение корня запущено. Следите за ним в разделе «Активность».",
  "rootChange.viewInActivity": "Открыть в разделе «Активность»",
  "rootChange.identityKeepsRoot":
    "Этот корень сохраняет свою идентичность и все {titles} привязок тайтлов.",
  "rootChange.identityKeepsDefault":
    "Он остаётся корнем по умолчанию для этой библиотеки.",
  "rootChange.identityLosesDefault":
    "Он больше не является корнем по умолчанию для этой библиотеки.",
  "rootChange.consolidationStatement":
    "Все {titles} тайтлов этого корня переезжают на выбранный вами корень.",
  "rootChange.consolidationRetiresConfiguration":
    "Настройка исходного корня выводится из обращения после того, как всё переехало и завершилась вся работа с корзиной.",
  "rootChange.defaultTransfers":
    "Корень назначения становится корнем по умолчанию для этой библиотеки, поэтому новый контент будет попадать туда.",
  "rootChange.defaultStays":
    "Корень по умолчанию для этой библиотеки не меняется.",
  "rootChange.accountingHeading": "Все тайтлы этого корня",
  "rootChange.accountingAssigned": "Привязано",
  "rootChange.accountingRelocating": "Переезжают",
  "rootChange.accountingCatalogOnly": "Только каталог",
  "rootChange.accountingBlocked": "Заблокировано",
  "rootChange.accountingDoesNotClose":
    "Значения не сходятся с итогом. Не подтверждайте этот план.",
  "rootChange.blockedTitlesHeading":
    "{count} тайтлов не могут войти в эту операцию.",
  "rootChange.blockedTitlesNoExclude":
    "Исправьте это перед запуском. Операция над всем корнем не может оставить ни одного тайтла.",
  "rootChange.groupsHeading": "Что делает это объединение",
  "rootChange.group.movingIntoUnusedFolders": "В свободные папки",
  "rootChange.group.mergingWithDestinationTitles": "Слияние",
  "rootChange.group.folderNameCollisions": "Имя папки занято",
  "rootChange.group.mediaCollisions": "Имя медиафайла занято",
  "rootChange.group.dedupEligibleFiles": "Возможно идентичны",
  "rootChange.group.companionCollisions": "Имя сопутствующего файла занято",
  "rootChange.group.untrackedSourceEntries": "Не отслеживается в источнике",
  "rootChange.renamedFoldersHeading": "Папки, которые меняют имя",
  "rootChange.renamedFoldersSampled": "Здесь показаны только первые папки.",
  "rootChange.contentHeading": "Что лежит на этом корне",
  "rootChange.contentManaged": "Отслеживаемые медиафайлы",
  "rootChange.contentCompanions": "Сопутствующие файлы",
  "rootChange.contentUnknown": "Необъяснённое",
  "rootChange.contentSampled": "Здесь показаны только первые записи.",
  "rootChange.unknownContentHelp":
    "{bytes} содержимого, которое ничем не отслеживается. Оно никогда не перемещается и не удаляется, поэтому старое расположение остаётся.",
  "rootChange.retirementHeading": "Старое расположение после операции",
  "rootChange.retirementEmptyDirectoriesOnly":
    "Удаляются только пустые каталоги: {count} шт.",
  "rootChange.retirementVerificationFirst":
    "Ничего не удаляется в источнике, пока копия в назначении не проверена.",
  "rootChange.retirementRetainedHeading":
    "Остаются вместе со своим содержимым:",
  "rootChange.retirementBlockedHeading":
    "Старое расположение сохраняется, пока не решено следующее:",
  "rootChange.retirementPermitted":
    "Ничто не мешает вывести старое расположение из обращения.",
  "rootChange.verificationStatement":
    "{files} файлов ({bytes}) полностью проверяются в назначении, прежде чем какой-либо источник попадёт в корзину.",
  "rootChange.insufficientSpace":
    "В назначении нет нужных для этого {required}.",
  "rootChange.typedConfirmationPrompt": "Введите фразу для подтверждения",
  "rootChange.reason.root_identity_retained":
    "Корень сохраняет свою идентичность; меняется только путь.",
  "rootChange.reason.catalog_only_root_change":
    "Отслеживаемых файлов нет, поэтому это переназначение только в каталоге.",
  "rootChange.reason.title_blocked_for_root_change":
    "Этот тайтл пока не может войти в изменение корня.",
  "rootChange.reason.unknown_root_content":
    "Содержимое на корне, которое не объясняется ни одним тайтлом.",
  "rootChange.reason.source_retirement_blocked":
    "Старое расположение пока нельзя вывести из обращения.",
  "rootChange.reason.file_outside_title_folder":
    "Отслеживаемый файл, лежащий вне папки своего тайтла.",
  "rootChange.reason.hardlinked_source":
    "У исходного файла есть другие жёсткие ссылки, которые не следуют за перемещением.",
  "rootChange.reason.roots_consolidated":
    "Все тайтлы этого корня переезжают на корень назначения.",
  "rootChange.reason.default_root_transferred":
    "Корень назначения становится корнем по умолчанию для этой библиотеки.",
  "rootChange.reason.moves_into_unused_folder":
    "Переезжает в папку, которую ничто не занимает.",
  "rootChange.reason.merges_with_destination_title":
    "Сливается с тайтлом назначения, с которым разделяет идентичность.",
  "rootChange.reason.folder_name_uniqued":
    "Имя папки занято в назначении, поэтому эта папка переименована.",
  "rootChange.reason.catalog_only_consolidation":
    "Отслеживаемых файлов нет, поэтому это переназначение только в каталоге.",
  "rootChange.reason.destination_folder_exists":
    "Папка назначения уже существует.",
  "rootChange.retirementBlocker.blocked_titles":
    "Некоторые тайтлы не могут войти в операцию.",
  "rootChange.retirementBlocker.unexplained_source_content":
    "Содержимое в источнике, которое ничем не объясняется.",
  "rootChange.refusal.root_change_path_not_absolute":
    "Укажите абсолютный путь.",
  "rootChange.refusal.root_change_paths_overlap":
    "Новый путь находится внутри старого или наоборот.",
  "rootChange.refusal.root_change_source_root_is_symlink":
    "Этот корень является символической ссылкой. Сначала укажите на реальный каталог.",
  "rootChange.refusal.root_change_source_root_unavailable":
    "Этот корень сейчас недоступен для чтения, поэтому его содержимое нельзя спланировать.",
  "rootChange.refusal.root_change_destination_not_empty":
    "По новому пути уже есть содержимое. Выберите пустой или новый каталог.",
  "rootChange.refusal.root_change_destination_parent_missing":
    "Родительский каталог нового пути не существует.",
  "rootChange.refusal.root_change_destination_is_configured_root":
    "Этот путь уже настроен как корень другой библиотеки.",
  "rootChange.refusal.root_change_destination_conflicts_with_recycle_bin":
    "Этот путь пересекается с пользовательской корзиной: он не должен содержать корзину или находиться внутри неё.",
  "rootChange.refusal.root_change_mode_not_supported":
    "Смена корня всегда перемещает файлы; вариант «файлы уже на месте» здесь не предлагается.",
  "rootChange.refusal.root_consolidation_path_not_absolute":
    "Укажите абсолютный путь.",
  "rootChange.refusal.root_consolidation_same_root":
    "Корень нельзя влить в самого себя.",
  "rootChange.refusal.root_consolidation_paths_overlap":
    "Один из этих корней находится внутри другого.",
  "rootChange.refusal.root_consolidation_source_root_is_symlink":
    "Этот корень является символической ссылкой. Сначала укажите на реальный каталог.",
  "rootChange.refusal.root_consolidation_source_root_unavailable":
    "Этот корень сейчас недоступен для чтения, поэтому его содержимое нельзя спланировать.",
  "rootChange.refusal.root_consolidation_destination_root_unavailable":
    "Корень назначения сейчас недоступен для чтения, поэтому нельзя учесть то, что в нём уже лежит.",
  "rootChange.refusal.root_consolidation_mode_not_supported":
    "Объединение всегда перемещает файлы; вариант «файлы уже на месте» здесь не предлагается.",
  "indexerSearch.searchWithThisIndexer": "Искать в этом индексаторе",
  "indexerSearch.queryPlaceholder": "Искать релиз по всем индексаторам…",
  "indexerSearch.kindLabel": "Тип",
  "indexerSearch.kind.movie": "Фильм",
  "indexerSearch.kind.series": "Сериал",
  "indexerSearch.kind.anime": "Аниме",
  "indexerSearch.kind.raw": "Прямой",
  "indexerSearch.scope.indexers": "Индексаторы",
  "indexerSearch.scope.indexersAll": "Все ({{count}})",
  "indexerSearch.scope.indexersSome": "Выбрано: {{count}}",
  "indexerSearch.scope.indexersAllOption": "Все индексаторы",
  "indexerSearch.scope.categories": "Категории",
  "indexerSearch.scope.categoriesDefault": "По умолчанию для типа",
  "indexerSearch.scope.categoriesHelp":
    "Идентификаторы категорий Newznab через запятую. Оставьте пустым, чтобы использовать значения по умолчанию для выбранного типа.",
  "indexerSearch.advanced": "Дополнительно",
  "indexerSearch.advanced.minSize": "Мин. размер",
  "indexerSearch.advanced.maxSize": "Макс. размер",
  "indexerSearch.advanced.minSeeders": "Мин. сидов",
  "indexerSearch.advanced.maxAge": "Макс. возраст",
  "indexerSearch.advanced.limit": "Лимит результатов",
  "indexerSearch.unit.gib": "ГиБ",
  "indexerSearch.unit.days": "дней",
  "indexerSearch.unit.perIndexer": "на индексатор",
  "indexerSearch.saveSearch": "Сохранённые",
  "indexerSearch.saveCurrentSearch": "Сохранить текущий поиск",
  "indexerSearch.savedSearchesEmpty": "Сохранённых поисков пока нет.",
  "indexerSearch.health.releases": "Релизов: {{count}}",
  "indexerSearch.health.indexers": "Индексаторов: {{answered}}/{{total}}",
  "indexerSearch.health.elapsed": "{{seconds}} с",
  "indexerSearch.health.stillSearching": "Ещё ищут: {{count}} из {{total}}",
  "indexerSearch.health.failed": "ошибка",
  "indexerSearch.health.coolingDown": "пауза",
  "indexerSearch.health.skipped": "пропущен",
  "indexerSearch.skipReason.indexerDisabled": "Индексатор отключён",
  "indexerSearch.skipReason.temporarilyDisabled": "Временно отключён",
  "indexerSearch.skipReason.temporarilyDisabledUntil":
    "Временно отключён до {{time}}",
  "indexerSearch.skipReason.backedOff": "Пауза после недавних ошибок",
  "indexerSearch.skipReason.backedOffUntil": "Пауза до {{time}}",
  "indexerSearch.skipReason.noTextSearch": "Не поддерживает текстовый поиск",
  "indexerSearch.health.retryFailed": "Повторить неудачные",
  "indexerSearch.refine.title": "Уточнить",
  "indexerSearch.refine.reset": "Сбросить",
  "indexerSearch.facet.protocol": "Протокол",
  "indexerSearch.facet.resolution": "Разрешение",
  "indexerSearch.facet.source": "Источник",
  "indexerSearch.facet.audio": "Звук и HDR",
  "indexerSearch.facet.flags": "Метки",
  "indexerSearch.facet.indexer": "Индексатор",
  "indexerSearch.facet.size": "Размер",
  "indexerSearch.facet.usenet": "Usenet",
  "indexerSearch.facet.torrent": "Torrent",
  "indexerSearch.facet.atmos": "Atmos",
  "indexerSearch.facet.dolbyVision": "Dolby Vision",
  "indexerSearch.facet.hdr": "HDR",
  "indexerSearch.facet.freeleech": "Freeleech",
  "indexerSearch.facet.proper": "Proper",
  "indexerSearch.sizeRange": "{{min}} – {{max}} ГиБ",
  "indexerSearch.results.matched": "Показано: {{count}}",
  "indexerSearch.sort.label": "Сортировка",
  "indexerSearch.column.release": "Релиз",
  "indexerSearch.column.indexer": "Индексатор",
  "indexerSearch.column.size": "Размер",
  "indexerSearch.column.age": "Возраст",
  "indexerSearch.column.peers": "Пиры",
  "indexerSearch.row.select": "Выбрать релиз",
  "indexerSearch.row.grab": "Забрать",
  "indexerSearch.row.download": "Скачать файл",
  "indexerSearch.download.unavailable":
    "Этот релиз — магнет-ссылка, скачиваемого файла нет",
  "indexerSearch.row.expand": "Показать подробности",
  "indexerSearch.row.collapse": "Скрыть подробности",
  "indexerSearch.row.grabs": "Загрузок: {{count}}",
  "indexerSearch.protocol.nzb": "NZB",
  "indexerSearch.protocol.torrent": "Torrent",
  "indexerSearch.protocol.http": "HTTP",
  "indexerSearch.detail.title": "Подробности релиза",
  "indexerSearch.detail.publishDate": "Опубликовано",
  "indexerSearch.detail.releaseGroup": "Релиз-группа",
  "indexerSearch.detail.openOnIndexer": "Открыть в индексаторе",
  "indexerSearch.detail.notAvailable": "Не указано",
  "indexerSearch.age.hours": "{{count}} ч",
  "indexerSearch.age.days": "{{count}} дн",
  "indexerSearch.age.months": "{{count}} мес",
  "indexerSearch.age.years": "{{count}} г",
  "indexerSearch.footer.selected": "Выбрано: {{count}}",
  "indexerSearch.footer.nothingQueued":
    "Ничего не ставится в очередь, пока вы не заберёте.",
  "indexerSearch.footer.grabSelected": "Забрать выбранные",
  "indexerSearch.footer.downloadSelected": "Скачать {{count}}",
  "indexerSearch.empty.prompt":
    "Введите запрос и выполните поиск по индексаторам.",
  "indexerSearch.empty.searching": "Идёт поиск по индексаторам…",
  "indexerSearch.empty.noResults": "Подходящих релизов нет.",
  "grabDialog.title.one": "Забрать релиз",
  "grabDialog.title.many": "Забрать релизов: {{count}}",
  "grabDialog.subtitle":
    "Выберите название в библиотеке, к которому относится этот захват. Scryer импортирует и переименует его под этим названием.",
  "grabDialog.summary.mix": "МИКС",
  "grabDialog.summary.mixed": "Выбрано релизов: {{count}}",
  "grabDialog.assign.label": "Привязать к названию",
  "grabDialog.assign.placeholder": "Поиск по названиям библиотеки…",
  "grabDialog.assign.count": "{{shown}} из {{total}} названий",
  "grabDialog.assign.loading": "Поиск названий…",
  "grabDialog.assign.empty": "Подходящих названий в библиотеке нет.",
  "grabDialog.assign.unlinked":
    "Забрать без привязки — сразу в клиент, сопоставить позже в разделе «Импорт»",
  "grabDialog.gap.missing": "не хватает: {{count}}",
  "grabDialog.gap.complete": "Полностью",
  "grabDialog.gap.wanted": "Требуется",
  "grabDialog.season": "Сезон",
  "grabDialog.episode": "Эпизод",
  "grabDialog.episodic.help":
    "Оставьте оба поля пустыми, и Scryer определит сезон и эпизод по имени релиза.",
  "grabDialog.episodic.incomplete":
    "Укажите сезон и эпизод вместе либо оставьте оба поля пустыми.",
  "grabDialog.client": "Клиент загрузки",
  "grabDialog.client.routed": "По привязке индексатора",
  "grabDialog.client.placeholder": "Выберите клиент загрузки",
  "grabDialog.client.none": "Нет ни одного включённого клиента загрузки.",
  "grabDialog.importPath": "Путь импорта",
  "grabDialog.importPath.clientDefault": "по умолчанию у клиента загрузки",
  "grabDialog.option.replace": "Заменить существующий файл этим релизом",
  "grabDialog.option.acknowledge":
    "Я понимаю, что этот релиз отклонён по причине {{codes}}",
  "grabDialog.footer.linked": "Импортируется как {{name}}",
  "grabDialog.footer.unlinked":
    "Ничего не будет привязано — сопоставьте позже в разделе «Импорт»",
  "grabDialog.footer.pickTitle":
    "Выберите название в библиотеке или заберите без привязки.",
  "grabDialog.cta.assign": "Забрать и привязать",
  "grabDialog.cta.assignAll": "Забрать и привязать все",
  "grabDialog.cta.unlinked": "Забрать без привязки",
  "grabDialog.conflict": "Для «{{name}}» уже идёт загрузка.",
  "grabDialog.status.unlinked": "«{{name}}» отправлен в {{client}}",
  "grabDialog.status.failed": "{{name}}: {{reason}}",
  "grabDialog.error.expired":
    "Этого релиза больше нет в активном поиске. Запустите поиск снова.",
  "grabDialog.error.additionalScope":
    "{{name}} охватывает больше одного эпизода, поэтому его нельзя поставить в очередь как дополнительный файл. Это возможно только для фильма или одного эпизода.",
  "settings.titleTags": "Теги",
  "settings.titleTagsExisting": "Заданные теги",
  "settings.titleTagsHelp":
    "Применить к тайтлу можно только теги, заданные здесь. Переименование переписывает каждый тайтл и профиль задержки, где он стоит; удаление убирает его со всех тайтлов.",
  "settings.titleTagsNone": "Теги ещё не заданы.",
  "settings.titleTagsLoadError": "Не удалось загрузить теги.",
  "settings.titleTagLabelLabel": "Метка",
  "settings.titleTagLabelPlaceholder": "keep",
  "settings.titleTagLabelHelp":
    "Сохраняется в нижнем регистре, лишние пробелы убираются. Не более 64 символов.",
  "settings.titleTagDescriptionLabel": "Описание",
  "settings.titleTagDescriptionPlaceholder":
    "Для чего нужен этот тег (необязательно)",
  "settings.titleTagTitleCount": "Тайтлы",
  "settings.titleTagCreate": "Задать тег",
  "settings.titleTagCreateNew": "Задать новый тег",
  "settings.titleTagEdit": "Изменить тег",
  "settings.titleTagSaved": "Тег сохранён.",
  "settings.titleTagSaveError": "Не удалось сохранить тег.",
  "settings.titleTagDeleteError": "Не удалось удалить тег.",
  "settings.titleTagDeleted": "{{label}} удалён с {{count}} тайтлов.",
  "settings.titleTagDeleteConfirmTitle": "Удалить тег",
  "settings.titleTagDeleteConfirm":
    "{{label}} стоит на {{count}} тайтлах. Удаление уберёт его со всех них и из любого профиля задержки, который по нему фильтрует. Правила, которые его называют, перестанут срабатывать.",
  "settings.titleTagLabelRequired": "Введите метку для тега.",
  "settings.titleTagLabelReserved":
    "Метка не может начинаться с scryer: — этот префикс зарезервирован для настроек, которые Scryer пишет сам.",
  "settings.titleTagLabelTooLong": "Метка не может быть длиннее 64 символов.",
  "settings.titleTagLabelInvalid":
    "Метка не может содержать управляющие символы.",
  "settings.titleTagRenameSummary":
    "Переименован в {{label}}. Переписано тайтлов: {{titles}}, фильмов сериала: {{seriesMovies}}, профилей задержки: {{delayProfiles}}.",
  "settings.titleTagRenameWarning":
    "Исходники правил никогда не переписываются, поэтому {{count}} ссылок в правилах всё ещё называют старую метку {{label}} и перестанут срабатывать: {{references}}.",
  "settings.titleTagReferenceMaintenanceRuleSets":
    "наборы правил обслуживания: {{count}}",
  "settings.titleTagReferenceReleaseRuleSets":
    "наборы правил релизов: {{count}}",
  "settings.titleTagReferenceManagedTagFilters":
    "управляемые фильтры тегов: {{count}}",
  "settings.titleTagReferenceRequestRuleSets":
    "наборы правил запросов: {{count}}",
  "title.tagsLabel": "Теги",
  "title.tagsNone": "Тегов нет",
  "title.tagsAdd": "Добавить тег…",
  "title.tagsAllApplied": "Все заданные теги уже применены",
  "title.tagsRemove": "Убрать {{label}}",
  "title.tagsEmptyRegistry":
    "Теги ещё не определены. Их задаёт администратор в «Настройки › Теги».",
  "title.bulkAddTags": "Добавить теги",
  "title.bulkRemoveTags": "Убрать теги",
  "title.catalogFilters.themes": "Темы",
  "title.catalogFilters.selectThemes": "Выберите темы...",
  "title.catalogFilters.userTags": "Теги",
  "title.catalogFilters.selectUserTags": "Выберите теги...",
  "settings.titleTagSeriesMovieCount": "Фильмы сериала",
  "settings.maintenanceActionAddTags": "Добавить теги",
  "settings.maintenanceActionRemoveTags": "Удалить теги",
  "settings.maintenanceRuleTags": "Теги",
  "settings.maintenanceRuleTagsNone": "Теги ещё не выбраны",
  "settings.maintenanceRuleTagsHelp":
    "Правило назначает эти теги каждому подходящему тайтлу. Можно выбрать только теги, заданные администратором в разделе «Настройки → Теги».",
  "settings.maintenanceCandidatesTagPatchAdd": "добавляет {{tags}}",
  "settings.maintenanceCandidatesTagPatchRemove": "удаляет {{tags}}",
  "settings.maintenanceTemplateTaggedForRemovalTitle": "Помечено к удалению",
  "settings.maintenanceTemplateTaggedForRemovalDescription":
    "Внимание: этот шаблон удаляет тайтл и его файлы. Он применяется ко всем тайтлам с тегом «remove» после недели ожидания. Достаточно назначить тег вручную, чтобы запустить отсчёт; снятие тега в течение недели отменяет удаление. Сначала создайте тег «remove» в разделе «Настройки → Теги».",
  "settings.maintenanceTemplateFlagForReviewTitle": "Пометить для проверки",
  "settings.maintenanceTemplateFlagForReviewDescription":
    "Назначает тег needs-review тайтлам, импортированным более года назад, чтобы человек решил, что с ними делать. Ничего не удаляется.",
  "mediaFile.allInfo": "Сведения",
  "mediaFile.noAnalysis":
    "Для этого файла ещё нет результатов встроенного анализа.",
  "mediaInfo.sectionFile": "Файл",
  "mediaInfo.sectionVideo": "Видео",
  "mediaInfo.sectionAudio": "Аудиодорожки",
  "mediaInfo.sectionSubtitles": "Дорожки субтитров",
  "mediaInfo.sectionCaptions": "Службы скрытых субтитров",
  "mediaInfo.sectionRelease": "Релиз",
  "mediaInfo.sectionChapters": "Главы",
  "mediaInfo.sectionAttachments": "Вложения",
  "mediaInfo.path": "Путь",
  "mediaInfo.size": "Размер",
  "mediaInfo.container": "Контейнер",
  "mediaInfo.duration": "Длительность",
  "mediaInfo.chapterCount": "Главы",
  "mediaInfo.scanStatus": "Состояние сканирования",
  "mediaInfo.role": "Назначение",
  "mediaInfo.added": "Добавлен",
  "mediaInfo.originalPath": "Исходный путь",
  "mediaInfo.releaseHash": "Хеш релиза",
  "mediaInfo.resolution": "Разрешение",
  "mediaInfo.videoCodec": "Кодек",
  "mediaInfo.profile": "Профиль",
  "mediaInfo.bitDepth": "Разрядность",
  "mediaInfo.hdr": "HDR",
  "mediaInfo.threeD": "3D",
  "mediaInfo.frameRate": "Частота кадров",
  "mediaInfo.videoBitrate": "Битрейт видео",
  "mediaInfo.pixelFormat": "Формат пикселей",
  "mediaInfo.fieldOrder": "Порядок полей",
  "mediaInfo.aspectRatio": "Соотношение сторон",
  "mediaInfo.rotation": "Поворот",
  "mediaInfo.color": "Основные цвета / передаточная функция / матрица",
  "mediaInfo.contentLight": "Уровни яркости MaxCLL / MaxFALL",
  "mediaInfo.masteringDisplay": "Яркость мастеринг-дисплея",
  "mediaInfo.dolbyVision": "Профиль Dolby Vision",
  "mediaInfo.sceneName": "Имя релиза сцены",
  "mediaInfo.releaseGroup": "Релиз-группа",
  "mediaInfo.sourceType": "Источник",
  "mediaInfo.parsedResolution": "Определённое разрешение",
  "mediaInfo.parsedVideoCodec": "Определённый видеокодек",
  "mediaInfo.parsedAudioCodec": "Определённый аудиокодек",
  "mediaInfo.edition": "Издание",
  "mediaInfo.indexer": "Индексатор",
  "mediaInfo.grabbedTitle": "Выбранный для загрузки релиз",
  "mediaInfo.grabbedAt": "Добавлен на загрузку",
  "mediaInfo.acquisitionScore": "Оценка при выборе релиза",
  "mediaInfo.scoringLog": "Журнал оценки",
  "mediaInfo.sectionReleaseListing": "Публикация релиза",
  "mediaInfo.listingPublishedAt": "Опубликован",
  "mediaInfo.listingAgeAtGrab": "Возраст при выборе релиза",
  "mediaInfo.listingAgeDays": "{{count}} дней",
  "mediaInfo.listingThumbsUp": "Голосов за",
  "mediaInfo.listingThumbsDown": "Голосов против",
  "mediaInfo.listingPasswordProtected": "Защищён паролем",
  "mediaInfo.listingIndexerLanguages": "Языки по данным индексатора",
  "mediaInfo.listingCapturedAt": "Сведения получены",
  "mediaInfo.colIndex": "#",
  "mediaInfo.colLanguage": "Язык",
  "mediaInfo.colCodec": "Кодек",
  "mediaInfo.colProfile": "Профиль",
  "mediaInfo.colChannels": "Каналы",
  "mediaInfo.colBitrate": "Битрейт",
  "mediaInfo.colSampleRate": "Частота дискретизации",
  "mediaInfo.colSampleDepth": "Разрядность звука",
  "mediaInfo.colRoles": "Назначение",
  "mediaInfo.colName": "Имя",
  "mediaInfo.inferredLanguageFootnote":
    "Определено по названию дорожки, так как у дорожки нет поля языка.",
  "mediaInfo.colForced": "Принудительно",
  "mediaInfo.colDefault": "По умолчанию",
  "mediaInfo.colStart": "Начало",
  "mediaInfo.colEnd": "Конец",
  "mediaInfo.colTitle": "Название",
  "mediaInfo.colType": "Тип",
  "mediaInfo.colSize": "Размер",
  "mediaInfo.roleDefault": "По умолчанию",
  "mediaInfo.roleForced": "Принудительно",
  "mediaInfo.roleOriginal": "Оригинал",
  "nav.lists": "Списки",
  "lists.commandDescription":
    "Отслеживайте публичные списки и чарты и автоматически добавляйте их тайтлы.",
  "lists.tab.public": "Публичные списки",
  "lists.tab.exclusions": "Исключения",
  "lists.exclusions.commandDescription":
    "Тайтлы, которые не может добавить ни один список.",
  "lists.action.enabled": "Отслеживать {{name}}",
  "lists.action.retry": "Повторить",
  "lists.action.syncAll": "Синхронизировать все",
  "lists.action.syncNow": "Синхронизировать сейчас",
  "lists.action.unfollow": "Перестать отслеживать",
  "lists.url.label": "Отслеживать список по URL",
  "lists.url.placeholder": "Вставьте URL списка или чарта",
  "lists.url.submit": "Проверить список",
  "lists.url.help": "Вставьте адрес списка от поддерживаемого провайдера.",
  "lists.url.recognized": "{{provider}} · {{list}}",
  "lists.url.unrecognized":
    "Scryer не может отслеживать этот адрес. Поддерживаемые списки есть в каталоге провайдеров ниже.",
  "lists.followed.heading": "Отслеживаемые списки",
  "lists.followed.empty": "Публичные списки пока не отслеживаются.",
  "lists.followed.emptyManager":
    "Публичных списков пока нет. Вставьте URL выше или выберите список в каталоге ниже.",
  "lists.table.list": "Список",
  "lists.table.coverage": "Покрытие",
  "lists.table.mode": "Режим",
  "lists.table.lastSync": "Последняя синхронизация",
  "lists.table.neverSynced": "Ещё не синхронизирован",
  "lists.catalog.heading": "Обзор списков",
  "lists.catalog.providers": "Провайдеры списков",
  "lists.catalog.empty": "Провайдеры списков не установлены.",
  "lists.catalog.follow": "Отслеживать",
  "lists.catalog.every": "Синхронизация каждые {{interval}}",
  "lists.catalog.auth.memberAccount": "Аккаунт участника",
  "lists.catalog.auth.serverKey": "API-ключ сервера",
  "lists.interval.minutes": "{{count}} мин",
  "lists.interval.hours": "{{count}} ч",
  "lists.interval.days": "{{count}} дн.",
  "lists.kind.movie": "Фильмы",
  "lists.kind.series": "Сериалы",
  "lists.kind.anime": "Аниме",
  "lists.mode.add": "Добавить",
  "lists.mode.hold": "Отложить до одобрения",
  "lists.mode.request": "Запросить",
  "lists.mode.discover": "Показывать только в «Обзоре»",
  "lists.modeHelp.add":
    "Добавляет новые тайтлы с отслеживанием, выбранным для каждого типа. Отслеживаемые тайтлы ищутся, неотслеживаемые — нет.",
  "lists.modeHelp.hold":
    "Создаёт ожидающий запрос для каждого нового тайтла, чтобы его кто-то одобрил.",
  "lists.modeHelp.request": "Запрашивает новые тайтлы от имени участника.",
  "lists.modeHelp.discover": "Записывает тайтлы списка, ничего не добавляя.",
  "lists.mode.discoverUnavailable":
    "Станет доступно, когда списки появятся в «Обзоре».",
  "lists.onLeave.keep": "Оставить тайтл",
  "lists.onLeave.log": "Записать в журнал",
  "lists.onLeave.unmonitor": "Снять тайтл с отслеживания",
  "lists.onLeave.tag": "Пометить тайтл тегом",
  "lists.syncState.ok": "Синхронизирован",
  "lists.syncState.new": "Ещё не синхронизирован",
  "lists.syncState.fail": "Ошибка синхронизации",
  "lists.syncState.off": "Приостановлен",
  "lists.membershipState.inLibrary": "В библиотеке",
  "lists.membershipState.added": "Добавлен",
  "lists.membershipState.requested": "Запрошен",
  "lists.membershipState.held": "Ждёт одобрения",
  "lists.membershipState.rejected": "Отклонён",
  "lists.membershipState.filtered": "Отфильтрован",
  "lists.membershipState.excluded": "Исключён",
  "lists.membershipState.unresolved": "Нет совпадения",
  "lists.membershipState.blockedPermission": "Не разрешено",
  "lists.membershipState.discover": "Только «Обзор»",
  "lists.membershipState.pending": "Ожидает",
  "lists.runOutcome.succeeded": "Успешно",
  "lists.runOutcome.failed": "Ошибка",
  "lists.runOutcome.skipped": "Пропущено",
  "lists.counts.inLibrary": "В библиотеке",
  "lists.counts.added": "Добавлено",
  "lists.counts.requested": "Запрошено",
  "lists.counts.held": "Отложено",
  "lists.counts.filtered": "Отфильтровано",
  "lists.counts.excluded": "Исключено",
  "lists.counts.unresolved": "Нет совпадения",
  "lists.counts.empty": "Ещё ничего не синхронизировано",
  "lists.follow.title": "Отслеживать список",
  "lists.follow.editTitle": "Изменить список",
  "lists.follow.submit": "Отслеживать список",
  "lists.follow.sourceHeading": "Параметры списка",
  "lists.follow.order": "Порядок списка",
  "lists.follow.orderHint": "Названия читаются из списка в этом порядке.",
  "lists.follow.orderUnset": "Порядок по умолчанию",
  "lists.follow.order.rank": "Собственный рейтинг списка",
  "lists.follow.order.added": "Дата добавления",
  "lists.follow.order.released": "Дата выхода",
  "lists.follow.name": "Название",
  "lists.follow.kinds": "Включать",
  "lists.follow.modeHeading": "Когда появляется новый тайтл",
  "lists.follow.routesHeading": "Куда попадают тайтлы",
  "lists.follow.routesHelp":
    "Каждому типу тайтлов нужна библиотека. Типы без библиотеки никогда не добавляются.",
  "lists.follow.filtersHeading": "Фильтры",
  "lists.follow.syncHeading": "Синхронизация",
  "lists.follow.maxPerSync": "Не больше тайтлов за синхронизацию",
  "lists.follow.maxPerSyncPlaceholder": "Без ограничения",
  "lists.follow.maxPerSyncHelp":
    "Тайтлы сверх лимита ждут следующей синхронизации.",
  "lists.follow.onLeave": "Когда тайтл покидает список",
  "lists.follow.onLeaveHelp":
    "Применяется только к тайтлам, добавленным этим списком, которых нет ни в одном другом списке. Списки никогда не удаляют тайтлы.",
  "lists.follow.interval": "Расписание задаёт провайдер: каждые {{interval}}.",
  "lists.follow.problem.name": "Укажите название списка.",
  "lists.follow.problem.kinds": "Включите хотя бы один тип тайтлов.",
  "lists.follow.problem.mode": "Выберите, что делать с новыми тайтлами.",
  "lists.follow.problem.route":
    "Выберите библиотеку для каждого включённого типа.",
  "lists.follow.problem.maxPerSync": "Лимит должен быть не меньше 1.",
  "lists.follow.problem.params": "Заполните обязательные параметры списка.",
  "lists.route.heading": "{{kind}}",
  "lists.route.noLibrary":
    "Вы не управляете ни одной библиотекой для этого типа тайтлов.",
  "lists.filter.rating": "Минимальный рейтинг",
  "lists.filter.ratingScale": "Источник рейтинга",
  "lists.filter.ratingPlaceholder": "Любой",
  "lists.filter.scale.tmdb": "TMDB",
  "lists.filter.scale.imdb": "IMDb",
  "lists.filter.releaseYear": "Год выхода",
  "lists.filter.yearFrom": "С",
  "lists.filter.yearTo": "По",
  "lists.filter.excludeGenres": "Пропускать жанры",
  "lists.filter.excludeGenresPlaceholder": "horror, documentary",
  "lists.filter.languages": "Языки оригинала",
  "lists.filter.languagesPlaceholder": "en, ja",
  "lists.filter.releasedOnly": "Только вышедшие тайтлы",
  "lists.filter.releasedOnlyHelp":
    "Пропускает ещё не вышедшие тайтлы. Они проверяются снова при каждой синхронизации.",
  "lists.preview.heading": "Предпросмотр",
  "lists.preview.nextSyncHeading": "Следующая синхронизация добавит",
  "lists.preview.run": "Предпросмотр",
  "lists.preview.notRun":
    "Запустите предпросмотр, чтобы увидеть, что добавит первая синхронизация.",
  "lists.preview.failed": "Не удалось построить предпросмотр списка.",
  "lists.preview.total": "В списке",
  "lists.preview.inLibrary": "В библиотеке",
  "lists.preview.wouldAdd": "Будет добавлено",
  "lists.preview.filtered": "Отфильтровано",
  "lists.preview.excluded": "Исключено",
  "lists.preview.unresolved": "Нет совпадения",
  "lists.preview.unresolvedHelp":
    "Для {{count}} тайтлов пока нет совпадения. Попытки повторяются при каждой синхронизации.",
  "lists.preview.wouldAddHeading":
    "Тайтлы, которые добавит следующая синхронизация",
  "lists.preview.nothingToAdd": "Сейчас добавлять нечего.",
  "lists.detail.coverage": "Покрытие",
  "lists.detail.settings": "Настройки",
  "lists.detail.interval": "Расписание",
  "lists.detail.nextSync": "Следующая синхронизация",
  "lists.detail.noCap": "Без ограничения",
  "lists.detail.openAtProvider": "Открыть у провайдера",
  "lists.detail.pausedUntil":
    "Провайдер попросил Scryer снизить частоту запросов. Синхронизация возобновится после {{time}}.",
  "lists.detail.titles": "Тайтлы ({{count}})",
  "lists.detail.title": "Тайтл",
  "lists.detail.state": "Состояние",
  "lists.detail.noTitles": "Тайтлы ещё не записаны.",
  "lists.detail.sourceRank": "Место в источнике",
  "lists.detail.pageRange": "Записи {{from}}–{{to}} из {{total}}",
  "lists.detail.previousPage": "Назад",
  "lists.detail.nextPage": "Далее",
  "lists.detail.history": "История синхронизаций",
  "lists.detail.noRuns": "Синхронизаций пока не было.",
  "lists.detail.runCounts":
    "{{total}} в списке · {{added}} добавлено · {{held}} отложено",
  "lists.unfollow.title": "Перестать отслеживать {{name}}?",
  "lists.unfollow.description":
    "Scryer перестанет синхронизировать этот список. Тайтлы останутся в библиотеке, а ожидающие запросы — в разделе «Запросы».",
  "lists.status.followed": "Список {{name}} отслеживается.",
  "lists.status.updated": "{{name}} сохранён.",
  "lists.status.unfollowed": "Список {{name}} больше не отслеживается.",
  "lists.status.syncQueued": "Синхронизация {{name}}.",
  "lists.status.syncAllQueued": "Синхронизация всех списков.",
  "lists.exclusions.addHeading": "Исключить тайтл",
  "lists.exclusions.titleLabel": "Тайтл",
  "lists.exclusions.yearLabel": "Год",
  "lists.exclusions.kindLabel": "Тип",
  "lists.exclusions.idsLabel": "Внешние ID",
  "lists.exclusions.idsHelp": "Через запятую, в формате источник:id.",
  "lists.exclusions.scopeLabel": "Применяется к",
  "lists.exclusions.scopeAll": "Все списки",
  "lists.exclusions.scopeList": "Только {{name}}",
  "lists.exclusions.add": "Исключить",
  "lists.exclusions.added": "{{name}} исключён из списков.",
  "lists.exclusions.removed": "Списки снова могут добавлять {{name}}.",
  "lists.exclusions.empty": "Исключённых тайтлов нет.",
  "lists.exclusions.created": "Добавлено",
  "lists.exclusions.remove": "Снять исключение для {{name}}",
  "lists.exclusions.removeTitle": "Снять исключение для {{name}}?",
  "lists.exclusions.removeDescription":
    "Списки смогут снова добавить этот тайтл при следующей синхронизации.",
  "lists.exclusions.deleteCheckbox": "Также исключить из списков",
  "lists.exclusions.deleteNoIds":
    "{{name}} удалён, но у него нет внешних ID для исключения из списков.",
  "lists.exclusions.deleteFailed":
    "{{name}} удалён, но исключить его из списков не удалось. Добавьте исключение на странице «Списки».",
  "lists.policy.heading": "Политика списков участников",
  "lists.policy.help":
    "Определяет, как личные списки участников превращаются в запросы.",
  "lists.policy.empty": "Нет участников.",
  "lists.policy.member": "Участник",
  "lists.policy.requests30d": "Запросы из списков (30 дней)",
  "lists.policy.policy": "Политика",
  "lists.policy.policyFor": "Политика списков для {{name}}",
  "lists.policy.value.auto": "Одобрять автоматически",
  "lists.policy.value.approval": "Требуется одобрение",
  "lists.policy.value.none": "Списки запрещены",
  "settings.notificationEvent.listTitleAdded": "Список добавил тайтл",
  "settings.notificationEvent.listRequestSubmitted":
    "Запрос из списка отправлен",
  "settings.notificationEvent.listItemHeld": "Элемент списка задержан",
  "settings.notificationEvent.listTitleLeft": "Тайтл выбыл из списка",
  "settings.notificationEvent.listSyncFailed": "Сбой синхронизации списка",
  "settings.notificationEvent.listUnfollowed": "Подписка на список отменена",
  "settings.refMaintFactsListsAddedByList":
    "Добавил ли объект в библиотеку список, а не человек",
  "settings.refMaintFactsListsOnEnabledList":
    "Содержит ли объект ещё хотя бы один включённый список, публичный или личный",
  "settings.refMaintFactsListsNames":
    "Названия включённых публичных списков, которые ещё содержат объект",
  "settings.refMaintFactsListsLeftAll":
    "Убрали ли объект все списки, которые когда-либо его содержали",
  "settings.refMaintFactsListsLeftAt": "Когда последний список убрал объект",
  "settings.refMaintFactsListsLastListName":
    "Публичный список, из которого объект выбыл последним",
  "settings.refMaintFactsListsAddedByListValue":
    "Истина, если тайтл создан синхронизацией списка; неизвестно с причиной list_store_unreadable, если хранилище списков не удалось прочитать",
  "settings.refMaintFactsListsOnEnabledListValue":
    "Истина, пока хотя бы один включённый список содержит объект",
  "settings.refMaintFactsListsNamesValue":
    "Только названия публичных списков, по алфавиту; личный список участника никогда не называется",
  "settings.refMaintFactsListsLeftAllValue":
    "Истина, когда все списки, содержавшие объект, его убрали; ложь, пока он есть хотя бы в одном списке или если его никогда не было в списках",
  "settings.refMaintFactsListsLeftAtValue":
    "Метка времени RFC3339; отсутствует с причиной not_left_every_list, пока объект есть хотя бы в одном списке",
  "settings.refMaintFactsListsLastListNameValue":
    "Название этого публичного списка; отсутствует с причиной no_public_list_left, если объект никогда не выбывал из публичного списка",
  "lists.providerSettings.heading": "Настройки провайдера",
  "lists.providerSettings.copy":
    "Общие для всех списков этого провайдера на этом сервере.",
  "lists.providerSettings.required": "Обязательно",
  "lists.providerSettings.secretStored": "Сохранено",
  "lists.providerSettings.secretEmpty": "Не задано",
  "lists.providerSettings.secretPlaceholder":
    "Оставьте пустым, чтобы сохранить текущее значение",
  "lists.providerSettings.clearSecret": "Очистить",
  "lists.providerSettings.keepSecret": "Оставить",
  "lists.providerSettings.save": "Сохранить настройки",
  "lists.providerSettings.saved": "Настройки провайдера сохранены.",
  "title.listProvenance.added": "Добавлено списком {{name}}",
  "title.listProvenance.left": "Выбыло из списка {{name}}",
  "lists.tab.personal": "Личные",
  "lists.accounts.heading": "Ваши аккаунты",
  "lists.accounts.copy":
    "Подключите аккаунт, чтобы следить за личными списками и списками к просмотру. Только вы видите свои аккаунты и личные списки.",
  "lists.accounts.connect": "Подключить",
  "lists.accounts.connected": "Подключён",
  "lists.accounts.reconnect": "Переподключить",
  "lists.accounts.manage": "Управление",
  "lists.accounts.manageTitle": "Управление аккаунтом {{provider}}",
  "lists.accounts.private":
    "Эти списки и сведения об аккаунте видны только вам.",
  "lists.accounts.waiting":
    "Завершите авторизацию в окне провайдера. Подтверждение проверяется автоматически.",
  "lists.accounts.linkFailed":
    "Не удалось подключить аккаунт. Начните подключение заново.",
  "lists.accounts.pollRetrying":
    "Не удалось проверить подтверждение. Повторяем попытки, пока сессия не истечёт.",
  "lists.accounts.originNotAllowed":
    "По этому адресу нельзя подключать аккаунты. Если Scryer доступен через опубликованный порт контейнера, обратный прокси или имя хоста, попросите администратора задать публичный URL Scryer; иначе откройте Scryer через localhost или адрес, на котором он слушает.",
  "lists.accounts.linkExpired":
    "Срок этой сессии истёк. Начните новое подключение на странице списков.",
  "lists.accounts.popupBlocked":
    "Разрешите всплывающие окна для подтверждения аккаунта, затем подключите его снова.",
  "lists.accounts.finishing": "Завершение подключения аккаунта…",
  "lists.accounts.linked": "Аккаунт подключён.",
  "lists.accounts.returnToLists": "Вернуться к личным спискам",
  "lists.accounts.noProviders": "Провайдеры личных списков не установлены.",
  "lists.accounts.ownedLists": "Ваши списки",
  "lists.accounts.statuses": "Доступные статусы: {{statuses}}",
  "lists.accounts.unlink": "Отключить аккаунт",
  "lists.accounts.confirmUnlink": "Подтвердить отключение",
  "lists.accounts.unlinkHelp":
    "Отключение удаляет подписки этого аккаунта. Файлы медиатеки сохраняются.",
  "lists.accounts.unlinked": "Аккаунт отключён.",
  "lists.accounts.personalRouting":
    "Личные списки принадлежат вам. Добавление и поиск требуют права управлять названиями в каждой выбранной медиатеке. Запросы следуют вашей политике списков и правилам запросов.",
  "lists.providerApps.heading": "Свой OAuth",
  "lists.providerApps.settingsLabel": "Списки · Свой OAuth",
  "lists.providerApps.copy":
    "Приложения проекта работают без регистрации. При желании зарегистрируйте собственное приложение Trakt, AniList или MyAnimeList для прямого подключения этого экземпляра.",
  "lists.providerApps.simklDefault": "Simkl использует приложение проекта.",
  "lists.providerApps.useOwn": "Использовать своё приложение",
  "lists.providerApps.clientId": "ID клиента",
  "lists.providerApps.clientSecret": "Секрет клиента",
  "lists.providerApps.traktSecretHint":
    "Для Trakt необязательно. Trakt выполняет вход участников через PKCE; секрет нужен только старым приложениям Trakt.",
  "lists.providerApps.redirectUri": "URI перенаправления",
  "lists.providerApps.redirectHelp":
    "Зарегистрируйте URL /lists/oauth/callback этого экземпляра у провайдера. Оставьте секрет пустым, чтобы сохранить прежнее значение.",
  "episodeAssign.action": "Переназначить",
  "episodeAssign.dialogTitle": "Назначить эпизоду",
  "episodeAssign.dialogDescription":
    "Выберите эпизод, к которому относится {{name}}. Scryer переименует файл для этого эпизода и переместит его на место.",
  "episodeAssign.pendingDescription":
    "Scryer не смог определить, какой это эпизод: {{name}}. Выберите эпизод, и Scryer импортирует файл туда.",
  "episodeAssign.unnamed": "этот файл",
  "episodeAssign.targetLabel": "Эпизод",
  "episodeAssign.loadingEpisodes": "Загрузка эпизодов…",
  "episodeAssign.loadFailed": "Не удалось загрузить эпизоды.",
  "episodeAssign.unchanged": "Файл уже назначен этому эпизоду.",
  "episodeAssign.apply": "Назначить и переименовать",
  "episodeAssign.originalReleaseName": "Исходное название релиза",
  "episodeAssign.applying": "Назначение…",
  "episodeAssign.replaceTitle": "У этого эпизода уже есть файл",
  "episodeAssign.replaceDescription":
    "После назначения этот файл станет основным файлом эпизода. Текущий файл останется на диске как дополнительный.",
  "episodeAssign.replaceApply": "Заменить и назначить",
  "episodeAssign.success": "Файл назначен и переименован.",
  "episodeAssign.failed": "Не удалось назначить файл.",
  "episodeAssign.seasonGroup": "Сезон {{season}}",
  "episodeAssign.filterPlaceholder": "Фильтр эпизодов…",
  "episodeAssign.filterLabel": "Фильтр эпизодов",
  "episodeAssign.listLabel": "Эпизоды",
  "episodeAssign.noMatches": "Подходящих эпизодов нет.",
  "episodeAssign.selectPlaceholder": "Выберите эпизод…",
  "episodeAssign.refusal.noEpisodes": "Выберите хотя бы один эпизод.",
  "episodeAssign.refusal.outsideTitle":
    "Этот эпизод относится к другому тайтлу.",
  "episodeAssign.refusal.primaryConflict":
    "У этого эпизода уже есть основной файл.",
  "episodeAssign.refusal.notEpisodic":
    "Эпизодам можно назначать только файлы сериалов и аниме.",
  "episodeAssign.refusal.unsupportedFile":
    "Этот файл нельзя перемещать между эпизодами. Файлы фильмов сериала и образы дисков остаются на месте.",
  "episodeAssign.refusal.destinationOccupied":
    "Имя, которое получается для этого эпизода, уже занято другим файлом. Переместите или удалите его и повторите попытку.",
  "episodeAssign.refusal.renameRefused":
    "Scryer не смог задать имя файла для этого эпизода. Проверьте папку библиотеки и шаблон именования тайтла.",
  "episodeAssign.refusal.renameFailed":
    "Не удалось переместить файл под новое имя. Файл и его эпизоды остались без изменений.",
  "pendingImports.reason.anime_numbering_ambiguous":
    "Нумерация эпизодов неоднозначна",
  "initialAuth.title": "Настройте доступ к Scryer",
  "initialAuth.description":
    "Прежде чем продолжить, выберите, нужен ли вход в систему для этого экземпляра.",
  "initialAuth.choose": "Как пользователи должны получать доступ к Scryer?",
  "initialAuth.passwordMode": "Требовать пароль администратора",
  "initialAuth.noAuthMode": "Продолжить без аутентификации",
  "initialAuth.environment":
    "Среда запуска ограничивает выбор режима аутентификации.",
  "initialAuth.admin":
    "Задайте пароль для admin. Используйте не менее {{minLength}} символов.",
  "initialAuth.password": "Новый пароль",
  "initialAuth.confirm": "Подтвердите пароль",
  "initialAuth.mismatch": "Пароли не совпадают.",
  "initialAuth.acknowledge":
    "Я понимаю, что любой, кто имеет доступ к этому экземпляру без аутентификации, может действовать как администратор. Существующие сетевые ограничения по-прежнему действуют.",
  "initialAuth.code": "Код настройки",
  "initialAuth.codeHelp":
    "Найдите одноразовый код в консоли сервера или в выводе контейнера/службы. В настольной версии откройте logs/console.log в папке данных Scryer. При перезапуске Scryer код заменяется.",
  "initialAuth.continue": "Сохранить пароль и продолжить",
  "initialAuth.continueWithout":
    "Подтвердить отключение аутентификации и продолжить",
  "initialAuth.saving": "Сохранение…",
  "initialAuth.loadFailed":
    "Не удалось загрузить состояние настройки аутентификации. Повторите попытку, чтобы продолжить.",
  "initialAuth.retry": "Повторить",
  "initialAuth.saveFailed": "Не удалось сохранить настройки аутентификации.",
  "initialAuth.required":
    "Завершите настройку аутентификации, прежде чем продолжить.",
  "initialAuth.alreadyCompleted":
    "Настройка аутентификации уже завершена. Обновите страницу, чтобы продолжить.",
  "initialAuth.replaceRequired":
    "Войдите с временным паролем администратора и замените его.",
  "initialAuth.invalidCode":
    "Введите код настройки из консоли сервера или, в настольной версии, из logs/console.log в папке данных Scryer.",
  "initialAuth.rateLimited":
    "Слишком много неверных кодов настройки. Подождите минуту и повторите попытку.",
  "initialAuth.modeConflict":
    "Этот выбор противоречит среде запуска или указанному паролю.",
  "initialAuth.acknowledgeRequired":
    "Подтвердите, что хотите продолжить без аутентификации.",
  "initialAuth.sessionExpired":
    "Срок действия этого временного сеанса истёк. Войдите снова, чтобы продолжить.",
  "initialAuth.replacementTitle": "Выберите новый пароль",
  "initialAuth.replacementDescription":
    "Для входа использован временный пароль администратора. Замените его другим паролем, известным только вам.",
  "initialAuth.showPasswords": "Показать пароли",
  "initialAuth.signOut": "Выйти",
  "error.reason.EXTERNAL_ACCOUNT_LINK_FAILED":
    "Не удалось привязать внешнюю учётную запись.",
  "error.reason.CATALOG_TAG_INVALID": "Недопустимый тег тайтла.",
  "error.reason.SETTINGS_DELAY_PROFILE_CATALOG_INVALID":
    "Недопустимый каталог профилей задержки.",
  "error.reason.SETTINGS_TRUSTED_PROXY_POLICY_INVALID":
    "Недопустимая политика доверенных прокси.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_INVALID":
    "Недопустимый профиль сидирования.",
  "error.reason.PLUGIN_DESCRIPTOR_SDK_INCOMPATIBLE":
    "Дескриптор плагина несовместим с этим SDK.",
  "error.reason.PLUGIN_DESCRIPTOR_HOST_PERMISSIONS_INVALID":
    "Дескриптор плагина запрашивает недопустимые разрешения для хостов.",
  "error.reason.HTTP_WEBSOCKET_API_KEY_UNSUPPORTED":
    "Ключи API не поддерживаются для подключений WebSocket.",
  "error.reason.HTTP_WEBSOCKET_CLIENT_PROOF_REQUIRED":
    "Для доступа к WebSocket без аутентификации требуется подтверждение от веб-клиента Scryer.",
  "error.reason.PLUGIN_ARTIFACT_DESTINATION_FORBIDDEN":
    "URL артефакта указывает на частный или иной запрещённый адрес назначения.",
  "error.reason.PLUGIN_ARTIFACT_HOST_UNAVAILABLE":
    "Не удалось определить адрес хоста артефакта.",
  "error.reason.INDEXER_CATEGORY_MISMATCH":
    "Категория индексатора не соответствует содержимому релиза.",
  "importHistory.skipReason.SCENE_SUBTITLE_PENDING":
    "Субтитры Scene ещё подготавливаются",
  "importHistory.skipReason.ALREADY_IMPORTED": "Уже импортировано",
  "importHistory.skipReason.DUPLICATE_FILE": "Дубликат файла",
  "importHistory.skipReason.POST_DOWNLOAD_RULE_BLOCKED":
    "Заблокировано правилом обработки после загрузки",
  "importHistory.skipReason.POLICY_MISMATCH":
    "Не соответствует политике импорта",
  "importHistory.skipReason.UNRESOLVED_IDENTITY":
    "Не удалось однозначно определить тайтл",
  "importHistory.skipReason.UNPARSEABLE_EPISODE":
    "Не удалось определить эпизод",
  "importHistory.skipReason.NO_VIDEO_FILES": "Видеофайлы не найдены",
  "importHistory.skipReason.UNWANTED_EXECUTABLES":
    "Загрузка содержит нежелательные исполняемые файлы",
  "importHistory.skipReason.DOWNLOAD_IN_PROGRESS": "Загрузка ещё не завершена",
  "importHistory.skipReason.SOURCE_CHANGED":
    "Источник изменился после проверки",
  "importHistory.skipReason.SOURCE_UNAVAILABLE": "Источник временно недоступен",
  "importHistory.skipReason.DISK_FULL": "Недостаточно места на диске",
  "importHistory.skipReason.PERMISSION_DENIED": "Доступ запрещён",
  "importHistory.skipReason.PASSWORD_REQUIRED": "Требуется пароль архива",
  "importHistory.skipReason.ARCHIVE_EXTRACTION_PLUGIN_REQUIRED":
    "Требуется средство распаковки архивов",
  "importHistory.skipReason.ARCHIVE_EXTRACTION_TIMED_OUT":
    "Истекло время ожидания распаковки архива",
  "error.reason.LIST_SOURCE_RATE_LIMITED":
    "Поставщик списков запросил снижение частоты запросов.",
  "error.reason.LIST_SOURCE_UNAVAILABLE":
    "Не удалось связаться с поставщиком списков.",
  "error.reason.LIST_SOURCE_FAILED":
    "Поставщик не вернул список в читаемом формате.",
  "error.fallback.accountSecurityVerificationFailed":
    "Проверка безопасности учётной записи не пройдена.",
  "error.fallback.connectedAppCouldNotBeRevoked":
    "Не удалось отозвать доступ подключённого приложения.",
  "error.fallback.connectedAppsCouldNotBeLoaded":
    "Не удалось загрузить подключённые приложения.",
  "error.fallback.connectionFailed": "Не удалось подключиться",
  "error.fallback.connectionSucceededButProwlarrDiscoveryCouldNotStart":
    "Подключение установлено, но не удалось запустить обнаружение Prowlarr",
  "error.fallback.connectionTestFailed": "Проверка подключения не пройдена",
  "error.fallback.embyConnectTestFailed": "Проверка Emby Connect не пройдена",
  "error.fallback.failedToCompleteSetup": "Не удалось завершить настройку",
  "error.fallback.failedToCreateLibrary": "Не удалось создать библиотеку",
  "error.fallback.failedToFinalizeImport": "Не удалось завершить импорт",
  "error.fallback.failedToLoadImportActivity":
    "Не удалось загрузить сведения об активности импорта.",
  "error.fallback.failedToLoadImportProgress":
    "Не удалось загрузить ход импорта",
  "error.fallback.failedToLoadPreview":
    "Не удалось загрузить предварительный просмотр",
  "error.fallback.failedToLoadProwlarrDiscoveryStatus":
    "Не удалось загрузить состояние обнаружения Prowlarr",
  "error.fallback.failedToLoadQueue": "Не удалось загрузить очередь.",
  "error.fallback.failedToLoadWarmupProgress":
    "Не удалось загрузить ход предварительной подготовки",
  "error.fallback.failedToRestartProwlarrDiscovery":
    "Не удалось перезапустить обнаружение Prowlarr",
  "error.fallback.failedToSave": "Не удалось сохранить",
  "error.fallback.failedToUpdateSponsorPreference":
    "Не удалось обновить настройку Sponsor.",
  "error.fallback.importFailed": "Не удалось выполнить импорт",
  "error.fallback.theCreateResultIsUncertainReloadOAuthApplicationsBeforeTryingAgain":
    "Результат создания неизвестен. Перед повторной попыткой обновите список приложений OAuth.",
  "error.fallback.thisOAuthRequestHasAnInvalidClientOrRedirectURI":
    "В этом запросе OAuth указан недопустимый клиент или URI перенаправления.",
  "error.fallback.unableToAuthorizeThisIntegration":
    "Не удалось авторизовать эту интеграцию.",
  "error.fallback.unableToCancelImport": "Не удалось отменить импорт.",
  "error.fallback.unableToCopyTheAPIKey": "Не удалось скопировать ключ API.",
  "error.fallback.unableToCreateAPIKey": "Не удалось создать ключ API.",
  "error.fallback.unableToDeleteOAuthApplication":
    "Не удалось удалить приложение OAuth.",
  "error.fallback.unableToLoadAPIKeys": "Не удалось загрузить ключи API.",
  "error.fallback.unableToLoadOAuthApplications":
    "Не удалось загрузить приложения OAuth.",
  "error.fallback.unableToRevokeAPIKey": "Не удалось отозвать ключ API.",
  "error.fallback.unableToSaveOAuthApplication":
    "Не удалось сохранить приложение OAuth.",
  "error.reason.ACQUISITION_ANOTHER_SEARCH_RUNNING":
    "Уже выполняется другой поиск. Дождитесь его завершения или отмените его, прежде чем запускать этот поиск.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_BUDGET_EXHAUSTED":
    "Исчерпан лимит обработки цикла перенаправлений артефакта загрузки.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_LIMIT_EXCEEDED":
    "Артефакт загрузки превысил лимит перенаправлений.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_LOOP":
    "Обнаружен цикл перенаправлений артефакта загрузки.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_SCHEME_UNSUPPORTED":
    "Для перенаправлений артефакта загрузки допустимы только URL HTTP(S) или magnet.",
  "error.reason.ACQUISITION_ARTIFACT_RESOLVER_UNAVAILABLE":
    "Загрузка артефактов не поддерживается: обработчик получения артефактов индексатора не подключён.",
  "error.reason.ACQUISITION_AUTOMATIC_SEARCH_REQUIRES_TITLE":
    "Для автоматического поиска требуется тайтл.",
  "error.reason.ACQUISITION_AUTOMATIC_SEARCH_REQUIRES_TITLE_CANNOT_SELECT_WANTED_ITEM":
    "Для автоматического поиска требуется тайтл; выбрать элемент из списка «Разыскиваемое» нельзя.",
  "error.reason.ACQUISITION_BANDWIDTH_LIMIT_NEGATIVE":
    "Ограничение пропускной способности должно быть не меньше нуля.",
  "error.reason.ACQUISITION_CANONICAL_DOWNLOAD_SUBMISSION_REQUIRES_DOWNLOAD_ID":
    "Для канонической отправки загрузки требуется идентификатор загрузки.",
  "error.reason.ACQUISITION_CLIENT_ID_REQUIRED":
    "Требуется идентификатор клиента.",
  "error.reason.ACQUISITION_CONNECTION_TEST_UNSUPPORTED":
    "Проверка подключения не поддерживается для этого типа клиента загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_REDIRECT_LOCATION_INVALID":
    "Недопустимый адрес перенаправления артефакта загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_REDIRECT_MISSING_LOCATION":
    "В перенаправлении артефакта загрузки отсутствует адрес назначения.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_REQUIRED":
    "У релиза нет артефакта для загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_TYPE_UNSUPPORTED":
    "URL загрузки определён, но результат не является NZB, URI magnet или torrent-файлом.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_URL_INVALID":
    "Недопустимый URL артефакта загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_BASE_URL_INVALID":
    "У клиента загрузки нет корректного базового URL.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_CONFIG_INVALID":
    "Недопустимый JSON конфигурации клиента загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND":
    "Не удалось найти конфигурацию клиента загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_CREDENTIALS_REQUIRED":
    "Клиенту загрузки (sabnzbd) требуется ключ API или имя пользователя и пароль.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_DISABLED":
    "Клиент загрузки отключён.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_HOST_REQUIRED":
    "Невозможно определить базовый URL из конфигурации — требуется хост.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_ITEM_NOT_FOUND":
    "Не удалось найти элемент клиента загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_NOT_FOUND":
    "Не удалось найти клиент загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_TYPE_NOT_FOUND":
    "Для этого типа не настроен ни один клиент загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_TYPE_UNSUPPORTED":
    "Этот тип клиента загрузки не поддерживается.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_UNAVAILABLE":
    "Клиент загрузки отсутствует или отключён.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_UPDATE_EMPTY":
    "Необходимо указать хотя бы одно поле конфигурации клиента загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_CREDENTIALS_FORBIDDEN":
    "URL не должен содержать встроенные учётные данные.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_HOST_REQUIRED":
    "URL должен содержать хост.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_INVALID": "Недопустимый URL.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_SCHEME_UNSUPPORTED":
    "URL должен использовать http или https.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_NOT_ABSOLUTE":
    "HTTP-загрузчику требуется абсолютный путь к каталогу загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_NOT_DIRECTORY":
    "Путь к каталогу загрузки не является каталогом.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_NOT_WRITABLE":
    "Каталог загрузки недоступен для записи.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_REQUIRED":
    "HTTP-загрузчику требуется каталог загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_UNREACHABLE":
    "Каталог загрузки недоступен.",
  "error.reason.ACQUISITION_DOWNLOAD_NOT_PAUSABLE":
    "Эту загрузку нельзя приостановить.",
  "error.reason.ACQUISITION_DOWNLOAD_NOT_RESUMABLE":
    "Эту загрузку нельзя возобновить.",
  "error.reason.ACQUISITION_DOWNLOAD_ORIGIN_MISMATCH":
    "Проксируемый URL загрузки не соответствует источнику назначенного индексатора.",
  "error.reason.ACQUISITION_DOWNLOAD_PROXY_DISABLED":
    "Прокси, назначенный клиенту загрузки, отключён.",
  "error.reason.ACQUISITION_DOWNLOAD_PROXY_NOT_FOUND":
    "Не удалось найти прокси загрузки.",
  "error.reason.ACQUISITION_DOWNLOAD_ROUTING_STALE":
    "Выбранный клиент загрузки больше не подходит. Обновите маршрутизацию и повторите попытку.",
  "error.reason.ACQUISITION_DOWNLOAD_SOURCE_NOT_NZB_URL":
    "Источник загрузки должен быть URL NZB.",
  "error.reason.ACQUISITION_DOWNLOAD_SOURCE_REQUIRED":
    "Для добавления загрузки в очередь требуется указание источника.",
  "error.reason.ACQUISITION_DOWNLOAD_SUBMISSION_NOT_FOUND":
    "Запись об отправке загрузки не найдена.",
  "error.reason.ACQUISITION_EPISODE_VALUE_INVALID":
    "Недопустимое значение эпизода.",
  "error.reason.ACQUISITION_HTTP_ARTIFACT_REDIRECT_SCHEME_UNSUPPORTED":
    "Для перенаправлений артефакта загрузки допустимы только URL HTTP(S).",
  "error.reason.ACQUISITION_HTTP_DOWNLOAD_ALREADY_SUBMITTED":
    "Эта загрузка уже отправлена в HTTP-загрузчик.",
  "error.reason.ACQUISITION_HTTP_DOWNLOAD_NOT_FOUND":
    "Не удалось найти HTTP-загрузку.",
  "error.reason.ACQUISITION_HTTP_QUEUE_NOT_WRITABLE":
    "Очередь HTTP-загрузчика недоступна для записи.",
  "error.reason.ACQUISITION_HTTP_QUEUE_UNREADABLE":
    "Невозможно проверить очередь HTTP-загрузчика.",
  "error.reason.ACQUISITION_HTTP_SETTING_OUT_OF_RANGE":
    "Значение настройки HTTP-загрузчика выходит за допустимые пределы.",
  "error.reason.ACQUISITION_ID_SEARCH_FACET_UNSUPPORTED":
    "Неподдерживаемый аспект поиска по ID.",
  "error.reason.ACQUISITION_INDEXER_ARTIFACT_SOURCE_REQUIRED":
    "Требуется источник артефакта индексатора.",
  "error.reason.ACQUISITION_INDEXER_CAPS_BASE_URL_INVALID":
    "Недопустимый base_url для описания возможностей индексатора.",
  "error.reason.ACQUISITION_INDEXER_CAPS_BASE_URL_REQUIRED":
    "Для обновления возможностей индексатора требуется base_url.",
  "error.reason.ACQUISITION_INDEXER_CAPS_PROXY_REQUIRED":
    "Для обновления возможностей индексатора не передан назначенный ему прокси; отправка запроса напрямую запрещена.",
  "error.reason.ACQUISITION_INDEXER_CONFIG_JSON_INVALID":
    "Недопустимый config_json индексатора.",
  "error.reason.ACQUISITION_INDEXER_CONFIG_NOT_FOUND":
    "Не удалось найти конфигурацию индексатора.",
  "error.reason.ACQUISITION_INDEXER_ERROR_CURSOR_INVALID":
    "Недопустимый курсор ошибок индексатора.",
  "error.reason.ACQUISITION_INDEXER_MAPPING_CHANGED":
    "Сопоставление индексатора изменилось; обновите маршрутизацию и повторите попытку.",
  "error.reason.ACQUISITION_INDEXER_NOT_FOUND": "Не удалось найти индексатор.",
  "error.reason.ACQUISITION_INDEXER_PROVIDER_UNSUPPORTED":
    "Этот поставщик индексатора не поддерживается.",
  "error.reason.ACQUISITION_INDEXER_PROXY_DISABLED":
    "Для этого индексатора прокси отключён.",
  "error.reason.ACQUISITION_INDEXER_PROXY_NOT_FOUND":
    "Не удалось найти прокси индексатора.",
  "error.reason.ACQUISITION_INDEXER_RESOLVED_DIFFERENT_ITEM":
    "Индексатор определил другой элемент; отправка отменена.",
  "error.reason.ACQUISITION_INDEXER_RESOLVED_DIFFERENT_PAGE":
    "Индексатор определил другую страницу; отправка отменена.",
  "error.reason.ACQUISITION_INDEXER_ROUTING_UNAVAILABLE":
    "Маршрутизация индексатора недоступна.",
  "error.reason.ACQUISITION_INDEXER_SETTINGS_STALE":
    "Настройки индексатора изменились во время проверки; обновите страницу и повторите попытку.",
  "error.reason.ACQUISITION_INDEXER_UPDATE_EMPTY":
    "Необходимо указать хотя бы одно поле конфигурации индексатора.",
  "error.reason.ACQUISITION_MAGNET_REQUIRES_TORRENT_PROTOCOL":
    "Для magnet требуется протокол torrent.",
  "error.reason.ACQUISITION_MAINTENANCE_SEARCH_DISPATCH_REQUIRES_ACCEPTED_RECEIPT_INTENT":
    "Для отправки поискового задания обслуживания требуется принятое намерение регистрации подтверждения.",
  "error.reason.ACQUISITION_MAINTENANCE_SEARCH_DISPATCH_REQUIRES_PREALLOCATED_JOB_RUN_ID":
    "Для отправки поискового задания обслуживания требуется заранее выделенный идентификатор запуска задания.",
  "error.reason.ACQUISITION_MATCHED_TITLE_NO_LONGER_EXISTS_NOT_FOUND":
    "Сопоставленный тайтл не найден.",
  "error.reason.ACQUISITION_MATCHING_EPISODES_PAUSED_ALREADY_DOWNLOADING":
    "Соответствующие эпизоды приостановлены или уже загружаются. Возобновите приостановленное получение или дождитесь завершения загрузки.",
  "error.reason.ACQUISITION_NO_ROUTABLE_DOWNLOAD_CLIENT":
    "Маршрутизация не допускает ни одного включённого совместимого клиента загрузки.",
  "error.reason.ACQUISITION_NZBGET_JOB_ID_INVALID":
    "Недопустимый ID задания nzbget.",
  "error.reason.ACQUISITION_NZBGET_METHOD_UNAVAILABLE":
    "NZBGet не поддерживает эту сигнатуру запроса.",
  "error.reason.ACQUISITION_NZBGET_QUEUE_ID_INVALID":
    "Недопустимый идентификатор очереди nzbget.",
  "error.reason.ACQUISITION_NZB_ARTIFACT_TOO_LARGE":
    "Полученный артефакт NZB превышает ограничение размера.",
  "error.reason.ACQUISITION_NZB_COMPRESSION_UNSUPPORTED":
    "Загруженные данные NZB являются файлом; в промежуточное хранилище можно поместить только несжатые NZB или NZB со сжатием gzip, zlib или zstd.",
  "error.reason.ACQUISITION_NZB_DECOMPRESSION_FAILED":
    "Не удалось распаковать сжатые данные NZB.",
  "error.reason.ACQUISITION_NZB_PAYLOAD_NOT_XML":
    "Загруженные данные NZB не похожи на xml.",
  "error.reason.ACQUISITION_NZB_ROOT_INVALID":
    "Корневой элемент загруженных данных NZB должен быть <nzb>.",
  "error.reason.ACQUISITION_NZB_TEXT_DECODE_FAILED":
    "Не удалось декодировать текст XML NZB.",
  "error.reason.ACQUISITION_NZB_TRAILING_CONTENT":
    "Загруженные данные NZB содержат содержимое за пределами корневого элемента.",
  "error.reason.ACQUISITION_NZB_XML_INVALID":
    "Загруженные данные NZB не являются корректным xml: неожиданный конец файла.",
  "error.reason.ACQUISITION_OUTPUT_FILENAME_INVALID":
    "Имя выходного файла должно быть простым именем файла без пути.",
  "error.reason.ACQUISITION_PASSWORD_RETRY_INPUT_REQUIRED":
    "Требуются ID задания и новый пароль.",
  "error.reason.ACQUISITION_PAUSED_CONTENT_COVERED_RELEASE":
    "Получение содержимого, входящего в этот релиз, приостановлено. Возобновите его перед поиском.",
  "error.reason.ACQUISITION_PAUSED_WHILE_SEARCH_WAITING":
    "Получение было приостановлено, пока этот поиск ожидал выполнения. Возобновите его перед поиском.",
  "error.reason.ACQUISITION_PROWLARR_AUTH_FAILED":
    "Prowlarr отклонил настроенные учётные данные.",
  "error.reason.ACQUISITION_PROWLARR_CONFIG_INVALID":
    "Недопустимая конфигурация Prowlarr.",
  "error.reason.ACQUISITION_PROWLARR_PARENT_SEARCH_UNSUPPORTED":
    "Родительские конфигурации Prowlarr предназначены только для управления; выполняйте поиск через синхронизированные дочерние индексаторы.",
  "error.reason.ACQUISITION_PROXY_CONFIG_NOT_FOUND":
    "Не удалось найти конфигурацию прокси.",
  "error.reason.ACQUISITION_PROXY_REPOSITORY_UNAVAILABLE":
    "Клиенту загрузки назначен прокси, но репозиторий прокси не подключён.",
  "error.reason.ACQUISITION_RECOVERY_SEARCH_JOB_NOT_FOUND":
    "Задание восстановительного поиска не найдено.",
  "error.reason.ACQUISITION_RELEASE_DOWNLOAD_URL_INVALID":
    "Недопустимый URL загрузки релиза.",
  "error.reason.ACQUISITION_RELEASE_FLAG_UNSUPPORTED":
    "Неподдерживаемый флаг релиза.",
  "error.reason.ACQUISITION_RELEASE_NAME_SOURCE_REQUIRED_INVALID":
    "Требуются имя и источник релиза; их размер должен быть ограничен.",
  "error.reason.ACQUISITION_RELEASE_PROTOCOL_REQUIRED":
    "Для выбранного клиента требуется протокол релиза.",
  "error.reason.ACQUISITION_RELEASE_SIZE_OUT_OF_RANGE":
    "Размер релиза не должен быть отрицательным.",
  "error.reason.ACQUISITION_RELEASE_URL_FLAG_LIST_EXCEEDS_INPUT_LIMIT":
    "URL релиза или список флагов превышает ограничение входных данных.",
  "error.reason.ACQUISITION_RELEASE_URL_INVALID":
    "URL релиза должен использовать HTTP, HTTPS или корректный magnet.",
  "error.reason.ACQUISITION_REPLACEMENT_PASSWORD_REQUIRED":
    "Требуется новый пароль.",
  "error.reason.ACQUISITION_RESOLVED_NZB_INVALID":
    "Получены некорректные байты NZB.",
  "error.reason.ACQUISITION_ROUTED_DOWNLOAD_CLIENT_NOT_FOUND":
    "Не удалось найти клиент загрузки, выбранный маршрутизацией.",
  "error.reason.ACQUISITION_SABNZBD_CREDENTIALS_REQUIRED":
    "SABnzbd требует ключ API или имя пользователя и пароль.",
  "error.reason.ACQUISITION_SCOPE_NOT_FOUND": "Область получения не найдена.",
  "error.reason.ACQUISITION_SEARCH_ACTOR_DISABLED":
    "Исполнитель поиска отключён.",
  "error.reason.ACQUISITION_SEARCH_ACTOR_NO_LONGER_EXISTS":
    "Исполнитель поиска больше не существует.",
  "error.reason.ACQUISITION_SEARCH_ACTOR_UNAVAILABLE":
    "Исполнитель поиска недоступен.",
  "error.reason.ACQUISITION_SEARCH_FACET_INVALID":
    "Недопустимый сохранённый аспект поиска.",
  "error.reason.ACQUISITION_SEARCH_FACET_REQUIRED": "Требуется аспект поиска.",
  "error.reason.ACQUISITION_SEARCH_FACET_UNSUPPORTED":
    "Неподдерживаемый аспект поиска.",
  "error.reason.ACQUISITION_SEARCH_RUNNING":
    "Поиск для получения уже выполняется.",
  "error.reason.ACQUISITION_SEARCH_VERSION_UNSUPPORTED":
    "Неподдерживаемая версия сохранённого поиска.",
  "error.reason.ACQUISITION_SEASON_EPISODE_INVALID":
    "Сезон и эпизод должны содержать числовые значения.",
  "error.reason.ACQUISITION_SEASON_EPISODE_REQUIRED":
    "Требуются сезон и эпизод.",
  "error.reason.ACQUISITION_SEASON_INVALID": "Сезон должен быть целым числом.",
  "error.reason.ACQUISITION_SEASON_PACK_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE_EXTERNAL_IDS":
    "У объекта поиска пакета сезона нет названия или внешних ID, пригодных для поиска.",
  "error.reason.ACQUISITION_SEASON_VALUE_INVALID":
    "Недопустимое значение сезона.",
  "error.reason.ACQUISITION_SEEDING_PROFILE_NAME_CONFLICT":
    "Профиль сидирования с таким именем уже существует.",
  "error.reason.ACQUISITION_SEEDING_PROFILE_NOT_FOUND":
    "Не удалось найти профиль сидирования.",
  "error.reason.ACQUISITION_SELECTED_LIBRARY_UNAVAILABLE_INCOMPATIBLE_ENDPOINT":
    "Выбранная библиотека недоступна или несовместима с этой конечной точкой.",
  "error.reason.ACQUISITION_SELECTED_SEASON_NO_LONGER_MATCHES_TITLE":
    "Выбранный сезон больше не соответствует этому тайтлу. Обновите данные тайтла и повторите попытку.",
  "error.reason.ACQUISITION_SERIES_MOVIE_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE":
    "У объекта поиска фильма сериала нет названия, пригодного для поиска.",
  "error.reason.ACQUISITION_STAGED_PAYLOAD_TOO_LARGE":
    "Объём данных превышает ограничение размера промежуточного хранилища.",
  "error.reason.ACQUISITION_TITLE_BEING_MOVED_SEARCH_UNAVAILABLE":
    "Этот тайтл перемещается. Поиск станет доступен после завершения перемещения.",
  "error.reason.ACQUISITION_TITLE_NOT_FOUND": "Тайтл не найден.",
  "error.reason.ACQUISITION_TITLE_NO_LONGER_EXISTS_NOT_FOUND":
    "Выбранный тайтл не найден.",
  "error.reason.ACQUISITION_TITLE_NO_NAME_EXTERNAL_IDS":
    "У тайтла нет названия или внешних ID.",
  "error.reason.ACQUISITION_TRANSPORT_PROXY_INVALID":
    "Недопустимая конфигурация назначенного транспортного прокси.",
  "error.reason.ACQUISITION_WANTED_ITEM_NOT_FOUND":
    "Элемент списка «Разыскиваемое» не найден.",
  "error.reason.ACQUISITION_WANTED_KIND_INVALID":
    "Недопустимый сохранённый тип разыскиваемого элемента.",
  "error.reason.ACQUISITION_WEAVER_JOB_IDENTITY_INVALID":
    "Недопустимые идентификационные данные задания Weaver.",
  "error.reason.ACQUISITION_WEAVER_JOB_ID_INVALID":
    "Недопустимый идентификатор задания Weaver.",
  "error.reason.API_KEY_API_KEYS_RESTRICTED_USERS_WHO_MANAGE_SYSTEM_SETTINGS":
    "Ключи API доступны только пользователям, управляющим системными настройками.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_DECLARATIONS_INVALID":
    "Объявления ключей API для разработки должны иметь уникальные идентификаторы поиска.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_DECLARATION_INVALID":
    "Недопустимое объявление ключа API для разработки.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_LOOKUP_ID_COLLIDES_USER_MANAGED_KEY":
    "Идентификатор поиска ключа API для разработки совпадает с идентификатором ключа, управляемого пользователем.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_LOOKUP_ID_DIFFERENT_SECRET":
    "Идентификатору поиска ключа API для разработки соответствует другой секрет.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_OWNER_DOES_NOT_EXIST":
    "Владелец ключа API для разработки не существует.",
  "error.reason.API_KEY_ENVIRONMENT_SEEDED_API_KEYS_MANAGED_SCRYER_DEV_API_KEYS":
    "Ключи API, заданные через окружение, управляются параметром SCRYER_DEV_API_KEYS.",
  "error.reason.API_KEY_EXPIRED_REVOKED":
    "Срок действия ключа API истёк или ключ отозван.",
  "error.reason.API_KEY_LABEL_CONTROL_CHARACTER":
    "Метка ключа API не должна содержать управляющие символы.",
  "error.reason.API_KEY_LABEL_REQUIRED": "Требуется метка ключа API.",
  "error.reason.API_KEY_LABEL_TOO_LONG":
    "Длина метки ключа API не должна превышать выбранное число символов.",
  "error.reason.API_KEY_OWNER_DISABLED": "Владелец ключа API отключён.",
  "error.reason.API_KEY_OWNER_NO_LONGER_EXISTS":
    "Владелец ключа API больше не существует.",
  "error.reason.API_KEY_VALUE_INVALID": "Недопустимый ключ API.",
  "error.reason.APPLICATION_UPGRADE_APPLICATION_UPDATE_NOTICE_UNAVAILABLE":
    "Уведомление об обновлении приложения недоступно.",
  "error.reason.APPLICATION_UPGRADE_BACKUP_ALREADY_EXISTS":
    "Перезапись существующей резервной копии приложения для выбранного элемента запрещена.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDTAG_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE":
    "ExpectedTag не соответствует текущему уведомлению об обновлении приложения.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDTAG_REQUIRED":
    "ExpectedTag не должен быть пустым.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDVERSION_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE":
    "ExpectedVersion не соответствует текущему уведомлению об обновлении приложения.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDVERSION_INVALID":
    "ExpectedVersion должен быть строго новее запущенной версии.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDVERSION_REQUIRED":
    "ExpectedVersion не должен быть пустым.",
  "error.reason.APPLICATION_UPGRADE_INSTALLATION_UNAVAILABLE":
    "Эта установка приложения не подходит для обновления.",
  "error.reason.APPLICATION_UPGRADE_INSTALLER_DIGEST_INVALID":
    "Недопустимая контрольная сумма установщика.",
  "error.reason.APPLICATION_UPGRADE_INSUFFICIENT_FREE_SPACE_APPLICATION_UPGRADE":
    "Недостаточно свободного места для обновления приложения: требуется выбранное число байтов, доступно выбранное число байтов.",
  "error.reason.APPLICATION_UPGRADE_JOB_ALREADY_RUNNING":
    "Задание обновления приложения уже выполняется.",
  "error.reason.APPLICATION_UPGRADE_JOURNAL_PHASE_UNSUPPORTED":
    "Неподдерживаемая фаза журнала обновления приложения для выбранного элемента.",
  "error.reason.APPLICATION_UPGRADE_JOURNAL_SCHEMA_UNSUPPORTED":
    "Неподдерживаемая схема журнала обновления приложения для выбранного элемента.",
  "error.reason.APPLICATION_UPGRADE_NO_APPLICATION_UPDATE_NOTICE_UNAVAILABLE":
    "Нет доступного уведомления об обновлении приложения.",
  "error.reason.APPLICATION_UPGRADE_RUNNING_EXECUTABLE_NO_PARENT_DIRECTORY":
    "У запущенного исполняемого файла нет родительского каталога.",
  "error.reason.APPLICATION_UPGRADE_RUN_NOT_FOUND":
    "Запуск обновления приложения не найден.",
  "error.reason.APPLICATION_UPGRADE_UPGRADE_MANIFEST_TAG_DOES_NOT_MATCH_EXPECTEDTAG":
    "Тег манифеста обновления не соответствует expectedTag.",
  "error.reason.APPLICATION_UPGRADE_UPGRADE_MANIFEST_VERSION_DOES_NOT_MATCH_EXPECTEDVERSION":
    "Версия манифеста обновления не соответствует expectedVersion.",
  "error.reason.AUTHORIZATION_ACTION_PERMISSION_DENIED":
    "У вас нет разрешения на выполнение этого действия.",
  "error.reason.AUTHORIZATION_LIBRARY_ACCESS_DENIED":
    "У вас нет доступа к этой библиотеке.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_CANNOT_REPLACE_EXISTING_MEDIA":
    "Добавление дополнительных файлов в очередь не позволяет заменять существующие медиафайлы.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_DOES_NOT_SUPPORT_COLLECTION_SCOPES_YET":
    "Добавление дополнительных файлов в очередь пока не поддерживает область коллекции.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_REQUIRES_TITLE_EPISODE_SCOPE":
    "Для добавления дополнительных файлов в очередь требуется область тайтла или эпизода.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_SUPPORTS_ONLY_TITLE_SINGLE_EPISODE_SCOPES":
    "Добавление дополнительных файлов в очередь поддерживает только область тайтла или отдельного эпизода.",
  "error.reason.CATALOG_ADDITIONAL_FILE_TITLE_QUEUEING_SUPPORTS_ONLY_MOVIE_TITLES":
    "Добавление дополнительных файлов тайтла в очередь поддерживается только для тайтлов фильмов.",
  "error.reason.CATALOG_BEST_RELEASE_SEARCH_UNSUPPORTED":
    "Поиск лучшего релиза не поддерживается для области коллекции.",
  "error.reason.CATALOG_BLOCKLIST_ENTRY_NOT_FOUND":
    "Запись списка блокировки не найдена.",
  "error.reason.CATALOG_CHANGING_TITLE_FACET_UNSUPPORTED":
    "Изменение аспекта тайтла не поддерживается, поскольку тайтлы нельзя перемещать между библиотеками.",
  "error.reason.CATALOG_COLLECTION_FIELD_REQUIRED":
    "Необходимо указать хотя бы одно поле коллекции.",
  "error.reason.CATALOG_COLLECTION_INDEX_REQUIRED":
    "Требуется индекс коллекции.",
  "error.reason.CATALOG_COLLECTION_NOT_FOUND": "Коллекция не найдена.",
  "error.reason.CATALOG_COLLECTION_TYPE_REQUIRED": "Требуется тип коллекции.",
  "error.reason.CATALOG_COLLECTION_TYPE_UNKNOWN": "Неизвестный тип коллекции.",
  "error.reason.CATALOG_CONFIGURED_QUALITY_PROFILE_MISSING_CATALOG":
    "Настроенный профиль качества для выбранного элемента из выбранного значения отсутствует в каталоге.",
  "error.reason.CATALOG_DELETE_PREVIEW_CONFIRMATION_REQUIRED_BEFORE_DELETING_FILES_DISK":
    "Перед удалением файлов с диска необходимо подтвердить предварительный просмотр удаления.",
  "error.reason.CATALOG_DOWNLOAD_ALREADY_QUEUED_TITLE":
    "Загрузка этого тайтла уже добавлена в очередь.",
  "error.reason.CATALOG_DOWNLOAD_CLIENT_DISABLED":
    "Клиент загрузки с выбранным значением отключён.",
  "error.reason.CATALOG_DOWNLOAD_CLIENT_DOES_NOT_EXIST":
    "Клиент загрузки с выбранным значением не существует.",
  "error.reason.CATALOG_DOWNLOAD_CLIENT_UNAVAILABLE":
    "Выбранный клиент загрузки не подходит.",
  "error.reason.CATALOG_EPISODE_FIELD_REQUIRED":
    "Необходимо указать хотя бы одно поле эпизода.",
  "error.reason.CATALOG_EPISODE_MISSING_EPISODE_NUMBER":
    "У эпизода отсутствует номер эпизода.",
  "error.reason.CATALOG_EPISODE_MISSING_SEASON_NUMBER":
    "У эпизода отсутствует номер сезона.",
  "error.reason.CATALOG_EPISODE_NOT_FOUND": "Эпизод не найден.",
  "error.reason.CATALOG_EPISODE_SEARCHES_REQUIRE_SEASON":
    "Для поиска эпизодов требуется сезон.",
  "error.reason.CATALOG_EPISODE_TYPE_REQUIRED": "Требуется тип эпизода.",
  "error.reason.CATALOG_EPISODE_TYPE_UNKNOWN": "Неизвестный тип эпизода.",
  "error.reason.CATALOG_EXISTING_DOWNLOAD_NO_LONGER_SAFE_REPLACE":
    "Существующую загрузку больше нельзя безопасно заменить.",
  "error.reason.CATALOG_ID_ALREADY_ASSIGNED_TITLE":
    "Запрошенный идентификатор с выбранным значением уже назначен тайтлу с выбранным значением.",
  "error.reason.CATALOG_INDEXER_UNKNOWN":
    "Неизвестный индексатор с выбранным значением.",
  "error.reason.CATALOG_INTERACTIVE_RELEASE_SEARCH_NOT_FOUND":
    "Интерактивный поиск релизов не найден.",
  "error.reason.CATALOG_LIBRARY_NOT_FOUND": "Библиотека не найдена.",
  "error.reason.CATALOG_LISTED_BOTH_ADD_REMOVE":
    "Выбранный элемент указан и для добавления, и для удаления; выберите одно действие.",
  "error.reason.CATALOG_MAGNET_LINK_NO_FILE_DOWNLOAD":
    "Запрошенное значение является magnet-ссылкой и не содержит файла для загрузки.",
  "error.reason.CATALOG_MEDIA_FILE_DELETION_ALREADY_RUNNING":
    "Для выбранного значения уже выполняется удаление медиафайла.",
  "error.reason.CATALOG_MEDIA_FILE_NOT_FOUND": "Медиафайл не найден.",
  "error.reason.CATALOG_METADATA_LANGUAGE_INVALID":
    "Язык метаданных должен быть одним из следующих: eng, spa, fra, deu, ita, por, kor, zho или jpn.",
  "error.reason.CATALOG_MINIMUM_AVAILABILITY_INVALID":
    "Минимальная доступность должна иметь значение announced, in_cinemas или released.",
  "error.reason.CATALOG_MINIMUM_AVAILABILITY_ONLY_VALID_MOVIE_TITLES":
    "Минимальная доступность применима только к тайтлам фильмов.",
  "error.reason.CATALOG_MOST_RELEASES_DOWNLOADED_ONCE":
    "Одновременно можно загружать не более выбранного числа релизов.",
  "error.reason.CATALOG_MOVIE_ENTITY_NOT_FOUND": "Сущность фильма не найдена.",
  "error.reason.CATALOG_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND":
    "В ответе с метаданными фильма отсутствует искомый тайтл.",
  "error.reason.CATALOG_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE":
    "Поиск фильмов не может включать сезон или эпизод.",
  "error.reason.CATALOG_MOVIE_TITLE_DOES_NOT_CANONICAL_FOLDER_PATH":
    "У тайтла фильма нет канонического пути к папке.",
  "error.reason.CATALOG_NOT_DEFINED_TAG":
    "Выбранный элемент не является заданным тегом; прежде чем его можно будет применить, администратор должен добавить его в настройках.",
  "error.reason.CATALOG_NO_QUALITY_PROFILE_NOT_CONFIGURED":
    "Профиль качества не настроен; выберите глобальный профиль либо профиль категории, библиотеки или тайтла.",
  "error.reason.CATALOG_NO_USABLE_DOWNLOAD_SOURCE":
    "У запрошенного значения нет пригодного источника загрузки.",
  "error.reason.CATALOG_PENDING_RENAME_REFERS_MISSING_TITLE_TAG":
    "Ожидающее переименование ссылается на отсутствующий тег тайтла с выбранным значением.",
  "error.reason.CATALOG_PRIMARY_MOVIE_FILE_ONLY_SET_MOVIE_TITLES_SERIES_MOVIE_FILES_EPISODE_FILES":
    "Основной файл фильма можно задать только для тайтлов фильмов, файлов фильмов сериала или файлов эпизодов.",
  "error.reason.CATALOG_PROVIDE_EXACTLY_ONE_TITLE_ID_QUERY":
    "Укажите ровно один параметр: идентификатор тайтла или поисковый запрос.",
  "error.reason.CATALOG_RELEASE_DOWNLOAD_REQUIRED":
    "Выберите хотя бы один релиз для загрузки.",
  "error.reason.CATALOG_RELEASE_NO_LONGER_SEARCH_NOT_FOUND":
    "Релиз больше не найден в этом поиске.",
  "error.reason.CATALOG_RELEASE_NO_PROTOCOL": "У релиза не указан протокол.",
  "error.reason.CATALOG_RELEASE_NO_USABLE_DOWNLOAD_SOURCE":
    "У релиза нет пригодного источника загрузки.",
  "error.reason.CATALOG_RELEASE_SIZE_DOES_NOT_MATCH_SIGNED_CANDIDATE":
    "Размер релиза не соответствует подписанному кандидату.",
  "error.reason.CATALOG_ROOTFOLDERID_MUST_REFERENCE_ROOT_TITLE_LIBRARY":
    "RootFolderId должен ссылаться на корневую папку библиотеки тайтла.",
  "error.reason.CATALOG_ROOT_REQUIRED":
    "В библиотеке тайтла должна быть хотя бы одна корневая папка.",
  "error.reason.CATALOG_SEARCH_QUERY_REQUIRED": "Требуется поисковый запрос.",
  "error.reason.CATALOG_SELECTED_FILE_OUTSIDE_TITLE_S_CANONICAL_MOVIE_FOLDER":
    "Выбранный файл находится за пределами канонической папки фильма этого тайтла.",
  "error.reason.CATALOG_SELECTED_RELEASE_FILES_EXCEED_64_MIB_DOWNLOAD_BUNDLE_LIMIT":
    "Выбранные файлы релиза превышают ограничение пакета загрузки в 64 MiB.",
  "error.reason.CATALOG_SERIES_MOVIE_DOES_NOT_BELONG_TITLE":
    "Фильм сериала не относится к тайтлу.",
  "error.reason.CATALOG_SERIES_MOVIE_LINK_NOT_FOUND":
    "Связь с фильмом сериала не найдена.",
  "error.reason.CATALOG_SERIES_MOVIE_NOT_FOUND": "Фильм сериала не найден.",
  "error.reason.CATALOG_SERIES_MOVIE_REQUIRED":
    "Необходимо указать хотя бы один фильм сериала.",
  "error.reason.CATALOG_SERIES_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE":
    "Поиск фильмов сериала не может включать сезон или эпизод.",
  "error.reason.CATALOG_SMG_ID_OUT_OF_RANGE":
    "Идентификатор Smg должен быть положительным.",
  "error.reason.CATALOG_SMG_NO_SERIES_NOT_FOUND":
    "Для запрошенного значения в SMG нет сериала с выбранным идентификатором тайтла.",
  "error.reason.CATALOG_SNAPSHOT_JSON_COLUMN_INVALID":
    "Недопустимый JSON снимка.",
  "error.reason.CATALOG_SNAPSHOT_TITLE_MISSING_AMBIGUOUS":
    "Тайтл в снимке отсутствует или определён неоднозначно.",
  "error.reason.CATALOG_SNAPSHOT_TITLE_NO_LONGER_EXISTS":
    "Тайтл из снимка больше не существует.",
  "error.reason.CATALOG_TAG_ADD_REMOVE_REQUIRED":
    "Необходимо указать хотя бы один тег для добавления или удаления.",
  "error.reason.CATALOG_TITLES_NO_SERIES_METADATA":
    "У запрошенного числа тайтлов нет метаданных сериала.",
  "error.reason.CATALOG_TITLE_CARRY_MOST_TAGS":
    "У тайтла может быть не более выбранного числа тегов; после этого изменения у него будет выбранное число тегов.",
  "error.reason.CATALOG_TITLE_FACET_UNSUPPORTED":
    "Неподдерживаемый аспект тайтла.",
  "error.reason.CATALOG_TITLE_FIELD_REQUIRED":
    "Необходимо указать хотя бы одно поле тайтла.",
  "error.reason.CATALOG_TITLE_IDENTITY_REQUIRED":
    "Требуются идентификационные данные тайтла.",
  "error.reason.CATALOG_TITLE_NAME_REQUIRED": "Требуется название тайтла.",
  "error.reason.CATALOG_TITLE_REQUIRED":
    "Необходимо указать хотя бы один тайтл.",
  "error.reason.CATALOG_TITLE_TAG_CHANGED_WHILE_RENAME_PENDING":
    "Тег тайтла с выбранным значением изменился, пока ожидалось его переименование.",
  "error.reason.CATALOG_TOO_MANY_CONCURRENT_INTERACTIVE_SEARCHES":
    "Слишком много одновременных интерактивных поисков.",
  "error.reason.CATALOG_TVDB_ID_INVALID":
    "Идентификатор Tvdb должен быть числовым.",
  "error.reason.CONFIGURATION_CANDIDATE_EXPECTED_STATES_REQUIRED":
    "При переходе кандидата необходимо указать ожидаемые состояния.",
  "error.reason.CONFIGURATION_MAINTENANCE_ATTEMPT_RESULT_REQUIRED":
    "При завершении шага действия обслуживания необходимо передать результат попытки.",
  "error.reason.CONFIGURATION_MAINTENANCE_CANDIDATE_ALREADY_ACTIVE":
    "У правила обслуживания уже есть активный кандидат для этого тайтла.",
  "error.reason.CONFIGURATION_MAINTENANCE_CANDIDATE_PAGE_TOO_LARGE":
    "Для хода последовательности обслуживания допускается не более 100 кандидатов на страницу.",
  "error.reason.CONFIGURATION_MAINTENANCE_COMPLETION_LIMIT_INVALID":
    "Недопустимый лимит завершения обслуживания.",
  "error.reason.CONFIGURATION_MAINTENANCE_DISPATCH_STATE_INVALID":
    "Отправка универсального задания обслуживания должна начинаться в состоянии dispatching.",
  "error.reason.CONFIGURATION_MAINTENANCE_FAILED_STEP_ID_REQUIRED":
    "В маркере сбоя последовательности обслуживания отсутствует идентификатор неудавшегося шага.",
  "error.reason.CONFIGURATION_MAINTENANCE_HELD_RELEASE_INVALID":
    "Недопустимое освобождение удерживаемой попытки обслуживания.",
  "error.reason.CONFIGURATION_MAINTENANCE_JOB_RUN_ID_REQUIRED":
    "Для принятых подтверждений заданий обслуживания требуется действительный идентификатор запуска задания.",
  "error.reason.CONFIGURATION_MAINTENANCE_MEMBERSHIP_TERMINAL":
    "Участие в последовательности обслуживания остаётся в конечном состоянии до подтверждённого отсутствия совпадения.",
  "error.reason.CONFIGURATION_MAINTENANCE_RECEIPT_EXPECTED_STATES_REQUIRED":
    "При переходе подтверждения задания обслуживания необходимо указать ожидаемые состояния.",
  "error.reason.CONFIGURATION_MAINTENANCE_RECEIPT_PAGE_TOO_LARGE":
    "Проекция подтверждений последовательности обслуживания допускает не более 700 шагов.",
  "error.reason.CONFIGURATION_MAINTENANCE_RELEASE_REASON_REQUIRED":
    "Для освобождения удерживаемой попытки обслуживания не указана причина.",
  "error.reason.CONFIGURATION_MAINTENANCE_RETRY_ATTEMPT_NOT_ADVANCED":
    "При повторном выполнении шага действия обслуживания необходимо увеличить номер попытки.",
  "error.reason.CONFIGURATION_MAINTENANCE_RULE_HAS_HISTORY":
    "У этого правила обслуживания есть активные кандидаты или история действий; вместо удаления отключите его.",
  "error.reason.CONFIGURATION_MAINTENANCE_SEQUENCE_IDENTITY_MISMATCH":
    "Шаг действия обслуживания не соответствует сохранённому идентификатору своей последовательности.",
  "error.reason.CONFIGURATION_MAINTENANCE_STEP_ATTEMPT_INVALID":
    "При захвате шага действия обслуживания должна запускаться выполняемая попытка с неотрицательным номером.",
  "error.reason.CONFIGURATION_MAINTENANCE_STEP_PREFIX_MISMATCH":
    "Маркер сбоя последовательности обслуживания не сохраняет префикс своих шагов.",
  "error.reason.CONFIGURATION_MAINTENANCE_STEP_TARGET_INVALID":
    "Захват шага действия обслуживания содержит недопустимые идентификационные данные цели.",
  "error.reason.CONFIGURATION_MAINTENANCE_SUCCESS_EVIDENCE_INCOMPLETE":
    "В маркере успешного завершения последовательности обслуживания отсутствует полный набор подтверждений шагов.",
  "error.reason.CONFIGURATION_MAINTENANCE_TERMINAL_EVIDENCE_MISMATCH":
    "Конечный маркер обслуживания не соответствует сохранённым подтверждениям шагов.",
  "error.reason.CONFIGURATION_MAINTENANCE_TERMINAL_STATE_MISMATCH":
    "Конечный маркер последовательности обслуживания не согласуется с конечным состоянием кандидата.",
  "error.reason.CONFIGURATION_MAINTENANCE_TERMINAL_STEP_IDS_INVALID":
    "Конечный маркер последовательности обслуживания содержит недопустимые ожидаемые идентификаторы шагов.",
  "error.reason.CONFIGURATION_PLUGIN_NOT_INSTALLED":
    "Плагин больше не установлен.",
  "error.reason.CONFIGURATION_PLUGIN_SETTINGS_PLUGIN_REQUIRED":
    "Для настроек плагина требуется установленный плагин.",
  "error.reason.CONFIGURATION_QUALITY_PROFILE_REPLACEMENT_SCOPE_REQUIRED":
    "Для замены профилей качества требуется область действия.",
  "error.reason.CONFIGURATION_QUALITY_PROFILE_SCOPE_REQUIRED":
    "Для получения списка профилей качества требуется область действия.",
  "error.reason.CONFIGURATION_RULE_COPY_SOURCE_NOT_USER_RULE":
    "Копируемые наборы правил должны быть обычными пользовательскими правилами.",
  "error.reason.CONFIGURATION_RULE_PACK_OWNERSHIP_MISMATCH":
    "Набор правил не принадлежит запрошенному пакету.",
  "error.reason.CONFIGURATION_SETTING_DELETE_IDENTITY_REQUIRED":
    "Для удаления переопределения настройки требуются область действия и key_name.",
  "error.reason.CONFIGURATION_SETTING_KEY_AMBIGUOUS":
    "В категориях найден неоднозначный ключ настройки «.».",
  "error.reason.CONFIGURATION_SETTING_READ_IDENTITY_REQUIRED":
    "Для чтения настройки требуются область действия и key_name.",
  "error.reason.CONFIGURATION_SETTING_UPDATE_IDENTITY_REQUIRED":
    "Для обновления настройки требуются область действия и key_name.",
  "error.reason.CONFIGURATION_SETTING_VALUE_EMPTY":
    "Значение настройки не может быть пустым.",
  "error.reason.CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND":
    "Не удалось найти конфигурацию поставщика субтитров.",
  "error.reason.CONFIGURATION_SUBTITLE_PROVIDER_UPDATE_EMPTY":
    "Необходимо указать хотя бы одно поле конфигурации поставщика субтитров.",
  "error.reason.CONFIGURATION_UNKNOWN_SETTING_KEY":
    "Неизвестный ключ настройки.",
  "error.reason.DATASTORE_BACKUP_BLOB_BASE64_INVALID":
    "Данные двоичного объекта резервной копии содержат некорректный base64.",
  "error.reason.DATASTORE_BACKUP_BLOB_BYTES_MISSING":
    "В данных двоичного объекта резервной копии отсутствуют байты в base64.",
  "error.reason.DATASTORE_BACKUP_BYTEA_UNSUPPORTED":
    "Строка резервной копии содержит неподдерживаемое значение PostgreSQL bytea.",
  "error.reason.DATASTORE_BACKUP_FOREIGN_KEY_INVALID":
    "Восстановленная база данных не прошла проверку внешних ключей.",
  "error.reason.DATASTORE_BACKUP_INTEGER_OUT_OF_RANGE":
    "Строка резервной копии содержит целое число вне диапазона SQLite i64.",
  "error.reason.DATASTORE_BACKUP_NUMBER_UNSUPPORTED":
    "Строка резервной копии содержит неподдерживаемое числовое значение.",
  "error.reason.DATASTORE_BACKUP_REQUIRED_COLUMN_MISSING":
    "В строке резервной копии отсутствует обязательный столбец.",
  "error.reason.DATASTORE_BACKUP_REQUIRED_COLUMN_NULL":
    "Строка резервной копии содержит null в обязательном столбце.",
  "error.reason.DATASTORE_BACKUP_ROW_IMPORT_FAILED":
    "Не удалось импортировать строку резервной копии.",
  "error.reason.DATASTORE_BACKUP_ROW_INVALID":
    "Недопустимая строка резервной копии.",
  "error.reason.DATASTORE_BACKUP_ROW_NOT_OBJECT":
    "Строка резервной копии должна быть объектом.",
  "error.reason.DATASTORE_BACKUP_ROW_UNREADABLE":
    "Не удалось прочитать строку резервной копии.",
  "error.reason.DATASTORE_BACKUP_TABLE_MISSING":
    "В резервной копии отсутствуют данные таблицы.",
  "error.reason.DATASTORE_BACKUP_TABLE_SET_MISMATCH":
    "Набор таблиц резервной копии не соответствует текущему каталогу восстановления.",
  "error.reason.DATASTORE_BACKUP_TIMESTAMP_BLANK":
    "Строка резервной копии содержит пустую обязательную метку времени.",
  "error.reason.DATASTORE_DATABASE_MIGRATION_CHECK_FAILED":
    "Проверка миграций базы данных не пройдена; есть ожидающие миграции.",
  "error.reason.DATASTORE_DIRECTORY_RESTORE_ENGINE_UNSUPPORTED":
    "Восстановление из подготовленного каталога резервной копии поддерживается только для PostgreSQL.",
  "error.reason.DATASTORE_PASSWORD_FILE_EMPTY":
    "SCRYER_DB_PASSWORD_FILE не содержит пароля.",
  "error.reason.DATASTORE_PASSWORD_FILE_UNREADABLE":
    "Не удалось прочитать SCRYER_DB_PASSWORD_FILE.",
  "error.reason.DATASTORE_POSTGRES_DATABASE_REQUIRED":
    "URL хранилища PostgreSQL должен содержать имя базы данных.",
  "error.reason.DATASTORE_POSTGRES_HOST_REQUIRED":
    "URL хранилища PostgreSQL должен содержать хост.",
  "error.reason.DATASTORE_POSTGRES_PASSWORD_INVALID":
    "Не удалось задать пароль PostgreSQL в URL хранилища.",
  "error.reason.DATASTORE_POSTGRES_PASSWORD_REQUIRED":
    "Для хранилища PostgreSQL требуется SCRYER_DB_PASSWORD, SCRYER_DB_PASSWORD_FILE или пароль в URL.",
  "error.reason.DATASTORE_POSTGRES_SSLMODE_REQUIRED":
    "В URL хранилища PostgreSQL необходимо явно указать sslmode.",
  "error.reason.DATASTORE_POSTGRES_SSLMODE_UNSUPPORTED":
    "Этот режим SSL PostgreSQL не поддерживается.",
  "error.reason.DATASTORE_POSTGRES_USERNAME_INVALID":
    "Не удалось задать имя пользователя PostgreSQL в URL хранилища.",
  "error.reason.DATASTORE_POSTGRES_USERNAME_REQUIRED":
    "Для хранилища PostgreSQL требуется SCRYER_DB_USER или имя пользователя в URL.",
  "error.reason.DATASTORE_RESTORED_EVENT_INVALID":
    "Не удалось закодировать данные восстановленного события предметной области.",
  "error.reason.DATASTORE_RESTORED_EXPLANATION_COMPACTION_FAILED":
    "Не удалось уплотнить восстановленное объяснение релиза.",
  "error.reason.DATASTORE_RESTORED_EXPLANATION_INVALID":
    "Не удалось закодировать восстановленное объяснение релиза.",
  "error.reason.DATASTORE_RESTORED_EXPLANATION_NULL":
    "Имеющееся восстановленное объяснение релиза закодировано как null.",
  "error.reason.DATASTORE_RESTORED_JSON_INVALID":
    "Восстановленное значение содержит некорректный JSON.",
  "error.reason.DATASTORE_RESTORED_REQUIRED_VALUE_NULL":
    "Обязательное восстановленное значение равно null.",
  "error.reason.DATASTORE_RESTORED_TABLE_ROW_COUNT_MISMATCH":
    "Число строк восстановленной таблицы не соответствует резервной копии.",
  "error.reason.DATASTORE_URL_INVALID": "Недопустимый SCRYER_DB_URL.",
  "error.reason.DATASTORE_URL_SCHEME_UNSUPPORTED":
    "URL хранилища должен использовать SQLite или PostgreSQL.",
  "error.reason.DISCOVERY_CONTEXT_CHANGE_TYPE_UNSUPPORTED":
    "Неподдерживаемый тип изменения контекста обнаружения с выбранным значением.",
  "error.reason.DISCOVERY_HOME_HERO_NOT_FOUND":
    "Главный баннер на главной странице рекомендаций не найден.",
  "error.reason.DISCOVERY_INCREMENTAL_RELOAD_RESOLVES_CHANGED_SUBJECTS_ABOVE_SMG_LIMIT":
    "При инкрементальном обновлении обнаружения получено выбранное число изменённых объектов, превышающее выбранный лимит SMG.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_INVALID_PREVIOUS_SUBJECT_JSON":
    "Ожидающее изменение обнаружения с выбранным значением содержит некорректный JSON предыдущего объекта.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_INVALID_RAW_SUBJECT_JSON":
    "Ожидающее изменение обнаружения с выбранным значением содержит некорректный JSON исходного объекта.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_MISSING_RAW_SUBJECT_JSON":
    "В ожидающем изменении обнаружения с выбранным значением отсутствует JSON исходного объекта.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_MISSING_SUBJECT_KEY":
    "В ожидающем изменении обнаружения с выбранным значением отсутствует ключ объекта.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_REMATCH_MISSING_PREVIOUS_SUBJECT_KEY":
    "В ожидающем повторном сопоставлении обнаружения с выбранным значением отсутствует ключ предыдущего объекта.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_REMOVAL_MISSING_SUBJECT_KEY":
    "В ожидающем удалении обнаруженного объекта с выбранным значением отсутствует ключ объекта.",
  "error.reason.DOWNLOAD_CLIENT_CONFIG_JSON_INVALID":
    "Недопустимый JSON конфигурации клиента загрузки.",
  "error.reason.DOWNLOAD_PATH_MAPPING_DOWNLOAD_CLIENT_CONFIG_JSON_INVALID":
    "Недопустимый JSON конфигурации клиента загрузки.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_INVALID":
    "Remote_path_mappings должен быть строкой.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_DUPLICATES_EXISTING_REMOTE_PATH_ROOT":
    "Строка сопоставления удалённых путей с выбранным значением дублирует существующий корень удалённого пути.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_INVALID":
    "Строка сопоставления удалённых путей с выбранным значением должна иметь формат REMOTE => LOCAL.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_LOCAL_PATH_INVALID":
    "В строке сопоставления удалённых путей с выбранным значением локальный путь должен быть абсолютным.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_END_WHITESPACE":
    "В строке сопоставления удалённых путей с выбранным значением удалённый путь не может заканчиваться пробельным символом.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_START_WHITESPACE":
    "В строке сопоставления удалённых путей с выбранным значением удалённый путь не может начинаться с пробельного символа.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REQUIRES_LOCAL_PATH":
    "В строке сопоставления удалённых путей с выбранным значением требуется локальный путь.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REQUIRES_REMOTE_PATH":
    "В строке сопоставления удалённых путей с выбранным значением требуется удалённый путь.",
  "error.reason.EVENTS_ACTIVE_PLUGIN_INSTALL_PROGRESS_NOT_FOUND":
    "Сведения о ходе активной установки плагина не найдены.",
  "error.reason.EVENTS_EPISODE_NOT_FOUND": "Эпизод не найден.",
  "error.reason.EVENTS_LIBRARY_ACCESS_DENIED":
    "У вас нет доступа к этой библиотеке.",
  "error.reason.EXTERNAL_ACCOUNT_IDENTITY_MISMATCH":
    "Внешняя учётная запись уже связана с другой идентичностью у поставщика.",
  "error.reason.EXTERNAL_ACCOUNT_LINKED_ELSEWHERE":
    "Эта учётная запись уже привязана к другому пользователю.",
  "error.reason.EXTERNAL_IMPORT_API_VERSION_EXPECTED_ONE_UNSUPPORTED":
    "Неподдерживаемая версия API выбранного поставщика для выбранного элемента; ожидалась одна из выбранных версий.",
  "error.reason.EXTERNAL_IMPORT_COULD_NOT_DETERMINE_VERSION":
    "Не удалось определить выбранную версию из выбранного элемента.",
  "error.reason.EXTERNAL_IMPORT_DID_NOT_REPORT_API_VERSION":
    "Запрошенный источник не сообщил версию API.",
  "error.reason.EXTERNAL_IMPORT_INVALID_HOST":
    "У запрошенного значения недопустимый хост.",
  "error.reason.EXTERNAL_IMPORT_SESSION_GONE":
    "Сеанс импорта больше не существует. Подключитесь снова.",
  "error.reason.EXTERNAL_IMPORT_SOURCE_APPLICATION_MISMATCH":
    "URL источника принадлежит другому приложению.",
  "error.reason.EXTERNAL_IMPORT_SOURCE_URL_INVALID":
    "Недопустимый URL источника.",
  "error.reason.EXTERNAL_IMPORT_SOURCE_URL_QUERY_OR_FRAGMENT":
    "Запрошенное значение не должно содержать строку запроса или фрагмент.",
  "error.reason.EXTERNAL_IMPORT_VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED":
    "Неподдерживаемая версия выбранного приложения для выбранного элемента; ожидалась выбранная основная версия или новее.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER":
    "Нельзя назначить корневую папку библиотеки папкой тайтла.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE":
    "Невозможно сравнить папку с выбранным значением с папкой с выбранным значением, принадлежащей тайтлу с выбранным значением.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_INSPECT_FOLDER_OWNED_ANOTHER_TITLE":
    "Невозможно проверить папку с выбранным значением, принадлежащую другому тайтлу.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_INSPECT_TITLE_FOLDER":
    "Невозможно проверить папку тайтла с выбранным значением.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_RESOLVE_TITLE_FOLDER":
    "Невозможно определить путь к папке тайтла с выбранным значением.",
  "error.reason.FOLDER_OWNERSHIP_FOLDER_ALREADY_OWNED_TITLE":
    "Папка с выбранным значением уже принадлежит тайтлу с выбранным значением.",
  "error.reason.FOLDER_OWNERSHIP_FOLDER_NOT_TITLE_BOUNDARY":
    "Записанная папка выбранного элемента является корнем библиотеки, а не папкой тайтла; сначала исправьте папку тайтла.",
  "error.reason.FOLDER_OWNERSHIP_FOLDER_OVERLAPS_FOLDER_OWNED_TITLE":
    "Папка с выбранным значением пересекается с папкой, принадлежащей тайтлу с выбранным значением.",
  "error.reason.FOLDER_OWNERSHIP_PATH_OUTSIDE_LIBRARY_ROOT":
    "Нельзя назначить тайтлу папку за пределами корней его библиотеки.",
  "error.reason.FOLDER_OWNERSHIP_RECYCLE_RESTORE_DESTINATION_OVERLAPS_ANOTHER_TITLE_S_OWNED_FOLDER":
    "Путь восстановления из корзины пересекается с папкой другого тайтла; файлы в корзине сохранены.",
  "error.reason.FOLDER_OWNERSHIP_RECYCLE_RESTORE_PATH_OUTSIDE_TITLE_S_CURRENT_OWNED_FOLDER":
    "Путь восстановления из корзины находится за пределами текущей папки тайтла; разрешите изменение папки перед восстановлением.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_ALREADY_OWNS_ANOTHER_FOLDER":
    "Тайтлу с выбранным значением уже принадлежит другая папка.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_FOLDER_INVALID":
    "Путь к папке тайтла должен быть абсолютным и не содержать переходов к родительским каталогам.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_FOLDER_NO_EXISTING_ANCESTOR":
    "У папки тайтла нет существующего родительского каталога ни на одном уровне.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_FOLDER_PATH_REQUIRED":
    "Требуется путь к папке тайтла.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_EMPTY":
    "Настроенный корневой путь медиафайлов с выбранным значением пуст.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_NOT_DIRECTORY":
    "Настроенный корневой путь медиафайлов с выбранным значением не является каталогом.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_SYMLINK":
    "Настроенный корневой путь медиафайлов с выбранным значением является символической ссылкой.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_UNAVAILABLE":
    "Настроенный корневой путь медиафайлов с выбранным значением недоступен.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_UNREADABLE":
    "Настроенный корневой путь медиафайлов с выбранным значением недоступен для чтения.",
  "error.reason.HTTP_API_EXPLORER_DISABLED": "Обозреватель API отключён.",
  "error.reason.HTTP_API_EXPLORER_MODE_DUPLICATE":
    "Повторяющийся режим доступа к обозревателю API.",
  "error.reason.HTTP_API_EXPLORER_MODE_INVALID":
    "Недопустимый режим доступа к обозревателю API.",
  "error.reason.HTTP_API_EXPLORER_SESSION_CHANGED":
    "Сеанс обозревателя API изменился.",
  "error.reason.HTTP_API_EXPLORER_WEB_SESSION_REQUIRED":
    "Для обозревателя API требуется веб-сеанс.",
  "error.reason.HTTP_AUTHORIZATION_HEADER_INVALID":
    "Недопустимый заголовок авторизации.",
  "error.reason.HTTP_BACKUP_BUNDLE_NOT_FOUND":
    "Не удалось найти пакет резервной копии.",
  "error.reason.HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID":
    "Недопустимая авторизация загрузки резервной копии.",
  "error.reason.HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_REQUIRED":
    "Требуется авторизация загрузки резервной копии.",
  "error.reason.HTTP_BACKUP_FILENAME_INVALID":
    "Недопустимое имя файла резервной копии.",
  "error.reason.HTTP_BACKUP_METADATA_NOT_FOUND":
    "Не удалось найти метаданные резервной копии.",
  "error.reason.HTTP_BACKUP_TICKET_QUERY_FORBIDDEN":
    "Передача билета загрузки резервной копии в строке запроса не допускается.",
  "error.reason.HTTP_RELEASE_DOWNLOAD_REQUEST_INVALID":
    "Недопустимый запрос загрузки релиза.",
  "error.reason.IDENTITY_ADMINISTRATOR_BOOTSTRAP_SEED_STALE":
    "Исходные данные администратора по умолчанию изменились во время начальной настройки.",
  "error.reason.IDENTITY_API_KEY_NOT_ENVIRONMENT_MANAGED":
    "Ключ API с этим идентификатором поиска не управляется через окружение.",
  "error.reason.IDENTITY_AUTHENTICATION_SESSION_INVALIDATED":
    "Сеанс аутентификации аннулирован.",
  "error.reason.IDENTITY_EMBY_ACCOUNT_DISABLED":
    "Учётная запись Emby отключена.",
  "error.reason.IDENTITY_EMBY_API_KEY_LIST_ACCESS_DENIED":
    "Администратор Emby не может получить список ключей API.",
  "error.reason.IDENTITY_EMBY_AVATAR_ID_INVALID":
    "Недопустимый идентификатор аватара Emby.",
  "error.reason.IDENTITY_EMBY_BASE_URL_INVALID":
    "Недопустимый базовый URL Emby.",
  "error.reason.IDENTITY_EMBY_BASE_URL_PATH_FORBIDDEN":
    "Базовый URL Emby не может содержать сегменты пути.",
  "error.reason.IDENTITY_EMBY_BASE_URL_UNSAFE":
    "Базовый URL Emby должен быть абсолютным URL HTTP(S) без учётных данных, строки запроса или фрагмента.",
  "error.reason.IDENTITY_EMBY_CONNECT_CREDENTIALS_INVALID":
    "Недопустимые учётные данные Emby Connect.",
  "error.reason.IDENTITY_EMBY_CONNECT_SERVER_ACCESS_DENIED":
    "Учётная запись Emby Connect не имеет доступа к настроенному серверу.",
  "error.reason.IDENTITY_EMBY_CONNECT_SIGN_IN_FAILED":
    "Не удалось войти через Emby Connect.",
  "error.reason.IDENTITY_EMBY_CREDENTIALS_INVALID":
    "Недопустимые учётные данные Emby.",
  "error.reason.IDENTITY_EMBY_CREDENTIAL_INVALID":
    "Недопустимые данные для аутентификации Emby.",
  "error.reason.IDENTITY_EMBY_ENDPOINT_URL_INVALID":
    "Недопустимый URL конечной точки Emby.",
  "error.reason.IDENTITY_EMBY_LOCAL_ADMINISTRATOR_REQUIRED":
    "Учётная запись Emby должна быть локальным администратором.",
  "error.reason.IDENTITY_EMBY_SERVER_IDENTITY_MISMATCH":
    "Идентификатор сервера Emby не соответствует сохранённому подключению.",
  "error.reason.IDENTITY_EMBY_SERVER_NOT_RECOGNIZED":
    "Указанный URL не является сервером Emby.",
  "error.reason.IDENTITY_EMBY_USER_LIST_ACCESS_DENIED":
    "Ключ API Emby недопустим или не позволяет получать список пользователей.",
  "error.reason.IDENTITY_EMBY_USER_VERIFICATION_FAILED":
    "Не удалось проверить пользователя Emby.",
  "error.reason.IDENTITY_EXTERNAL_ACCOUNT_IDENTITY_REQUIRED":
    "При привязке должны быть указаны идентификационные данные внешней учётной записи.",
  "error.reason.IDENTITY_EXTERNAL_ACCOUNT_NOT_FOUND":
    "Не удалось найти внешнюю учётную запись.",
  "error.reason.IDENTITY_JELLYFIN_ADMINISTRATOR_REQUIRED":
    "Для создания ключа API Scryer учётная запись Jellyfin должна иметь права администратора.",
  "error.reason.IDENTITY_JELLYFIN_ADMIN_CREDENTIALS_INVALID":
    "Недопустимые учётные данные администратора Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_LIST_ACCESS_DENIED":
    "Токен администратора Jellyfin не позволяет получать список ключей API.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_UNAVAILABLE":
    "Ключ API Jellyfin недоступен.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_URL_INVALID":
    "Недопустимый URL ключа API Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_USER_LIST_ACCESS_DENIED":
    "Ключ API Jellyfin недопустим или не даёт доступа к списку пользователей.",
  "error.reason.IDENTITY_JELLYFIN_AUTHENTICATION_URL_INVALID":
    "Недопустимый URL аутентификации Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_BASE_URL_INVALID":
    "Недопустимый базовый URL Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_BASE_URL_QUERY_FRAGMENT_FORBIDDEN":
    "Базовый URL подключения Jellyfin не должен содержать строку запроса или фрагмент.",
  "error.reason.IDENTITY_JELLYFIN_CREDENTIALS_INVALID":
    "Недопустимые учётные данные Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_SERVER_NOT_RECOGNIZED":
    "Сервер по указанному URL не представился сервером Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_SYSTEM_INFO_URL_INVALID":
    "Недопустимый URL системной информации Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_USERS_URL_INVALID":
    "Недопустимый URL списка пользователей Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_USER_ID_INVALID":
    "Недопустимый ID пользователя Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_USER_LIST_ACCESS_DENIED":
    "Ключ API Jellyfin недопустим или не позволяет получать список пользователей.",
  "error.reason.IDENTITY_JELLYFIN_USER_URL_INVALID":
    "Недопустимый URL пользователя Jellyfin.",
  "error.reason.IDENTITY_JELLYFIN_USER_VERIFICATION_FAILED":
    "Не удалось проверить пользователя Jellyfin.",
  "error.reason.IDENTITY_LAST_SIGN_IN_METHOD_REQUIRED":
    "Нельзя удалить последний доступный способ входа.",
  "error.reason.IDENTITY_LOGIN_VERIFICATION_CHALLENGE_NOT_FOUND":
    "Не удалось найти запрос проверки входа.",
  "error.reason.IDENTITY_MEDIA_SERVER_BASE_URL_INVALID":
    "Недопустимый базовый URL медиасервера.",
  "error.reason.IDENTITY_MEDIA_SERVER_REFRESH_API_KEY_REQUIRED":
    "Для обновления медиасервера требуется ключ API.",
  "error.reason.IDENTITY_MEDIA_SERVER_SCAN_API_KEY_REQUIRED":
    "Для сканирования каталога медиасервера требуется ключ API.",
  "error.reason.IDENTITY_MEDIA_SERVER_SIGNAL_USER_ID_REQUIRED":
    "Для чтения сигнала медиасервера требуется идентификатор пользователя у поставщика.",
  "error.reason.IDENTITY_OAUTH_CLIENT_DISABLED_UNAVAILABLE":
    "Клиент OAuth отключён или недоступен.",
  "error.reason.IDENTITY_OAUTH_GRANT_NOT_FOUND":
    "Не удалось найти разрешение OAuth.",
  "error.reason.IDENTITY_PASSKEY_DELETION_SESSION_STALE":
    "Сеанс удаления ключа доступа больше не актуален.",
  "error.reason.IDENTITY_PASSKEY_ENROLLMENT_SESSION_STALE":
    "Сеанс регистрации ключа доступа больше не актуален.",
  "error.reason.IDENTITY_PASSWORD_CHANGE_NOT_REQUIRED":
    "Смена пароля больше не требуется.",
  "error.reason.IDENTITY_PLEX_AUTH_TOKEN_INVALID":
    "Недопустимый токен аутентификации Plex.",
  "error.reason.IDENTITY_PLEX_AUTH_TOKEN_REQUIRED":
    "Требуется токен аутентификации Plex.",
  "error.reason.IDENTITY_PLEX_REFRESH_SERVER_REQUIRED":
    "Для обновления Plex необходимо выбрать сервер.",
  "error.reason.IDENTITY_PLEX_SCAN_SERVER_REQUIRED":
    "Для сканирования каталога Plex необходимо выбрать сервер.",
  "error.reason.IDENTITY_PLEX_SERVER_ACCESS_DENIED":
    "Учётная запись Plex не имеет доступа к настроенному серверу.",
  "error.reason.IDENTITY_PLEX_USER_LIST_ACCESS_DENIED":
    "Токен Plex недопустим или не позволяет получать список пользователей.",
  "error.reason.IDENTITY_RECOVERY_CODE_SESSION_STALE":
    "Сеанс замены кодов восстановления больше не актуален.",
  "error.reason.IDENTITY_TOTP_CHALLENGE_NOT_FOUND":
    "Не удалось найти запрос TOTP.",
  "error.reason.IDENTITY_TOTP_CREDENTIAL_NOT_FOUND":
    "Не удалось найти данные для аутентификации TOTP.",
  "error.reason.IDENTITY_TOTP_DISABLEMENT_SESSION_STALE":
    "Сеанс отключения TOTP больше не актуален.",
  "error.reason.IDENTITY_TOTP_ENROLLMENT_SESSION_STALE":
    "Сеанс настройки TOTP больше не актуален.",
  "error.reason.IDENTITY_USER_NOT_FOUND": "Не удалось найти пользователя.",
  "error.reason.IDENTITY_USER_UI_SETTINGS_NOT_FOUND":
    "Не удалось найти настройки интерфейса пользователя.",
  "error.reason.IDENTITY_WEBAUTHN_CHALLENGE_NOT_FOUND":
    "Не удалось найти запрос webauthn.",
  "error.reason.IDENTITY_WEBAUTHN_CREDENTIAL_NOT_FOUND":
    "Не удалось найти данные для аутентификации webauthn.",
  "error.reason.IMPORT_ACTIVE_IMPORT_STREAM_NOT_FOUND":
    "Активный поток импорта не найден.",
  "error.reason.IMPORT_ARCHIVE_DISCOVERY_DIRECTORY_DEPTH_LIMIT_EXCEEDED":
    "При поиске архивов превышено ограничение числа каталогов или глубины вложенности.",
  "error.reason.IMPORT_ARCHIVE_DISCOVERY_ENTRY_LIMIT_EXCEEDED":
    "При поиске архивов превышено ограничение числа элементов.",
  "error.reason.IMPORT_ARCHIVE_ENTRY_ESCAPES_EXTRACTION_DIRECTORY":
    "Элемент архива выходит за пределы каталога распаковки.",
  "error.reason.IMPORT_ARCHIVE_EXTRACTION_REQUIRES_RESOLVED_IMPORT_DESTINATION_BEFORE_STAGING_OUTPUT":
    "Перед размещением результатов распаковки выбранного значения в промежуточном хранилище необходимо определить путь назначения импорта.",
  "error.reason.IMPORT_ARCHIVE_HOLDS_ARCHIVES_NESTED_MORE_LEVELS_DEEP":
    "Архив содержит вложенные архивы с глубиной более выбранного числа уровней.",
  "error.reason.IMPORT_ARCHIVE_HOLDS_MORE_NESTED_ARCHIVES":
    "Архив содержит больше выбранного числа вложенных архивов.",
  "error.reason.IMPORT_ARCHIVE_MATCHED_TITLE_BUT_EXTRACTED_WORKSPACE_COULD_NOT_MOVED_WITHOUT_COPYING":
    "Архив сопоставлен с выбранным тайтлом, но распакованную рабочую папку с выбранным значением не удалось переместить в выбранное место без копирования.",
  "error.reason.IMPORT_ARCHIVE_OUTPUT_DIRECTORY_NOT_INSIDE_STAGING_WORKSPACE":
    "Выходной каталог архива с выбранным значением находится за пределами промежуточной рабочей папки.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_EXTRACTION":
    "Плагин архивов не поддерживает распаковку выбранного формата.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_MANIFEST_OUTPUT_NOT_REGULAR_FILE":
    "Выходной файл манифеста плагина архивов не является обычным файлом.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_MANIFEST_SIZE_MISMATCH":
    "Размер выбранного файла не соответствует манифесту плагина архивов.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_SYMLINK":
    "Результат работы плагина архивов содержит символическую ссылку.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_DIRECTORIES":
    "Результат работы плагина архивов содержит слишком много каталогов.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_ENTRIES":
    "Результат работы плагина архивов содержит слишком много элементов.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_FILES":
    "Результат работы плагина архивов содержит слишком много файлов.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_EXCEEDS_BYTES":
    "Результат работы плагина архивов превышает выбранное число байтов.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_NOT_REGULAR_FILE":
    "Результат работы плагина архивов не является обычным файлом.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_TOO_LARGE":
    "Результат работы плагина архивов слишком велик.",
  "error.reason.IMPORT_ARCHIVE_REEXTRACTION_FAILED":
    "Архив сопоставлен с выбранным тайтлом, но повторно распаковать его в папку назначения этого тайтла не удалось.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_LIES_INSIDE_OWN_EXTRACTION_OUTPUT":
    "Исходный архив с выбранным значением находится внутри собственного результата распаковки.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_METADATA_ESCAPES_SOURCE_DIRECTORY":
    "Метаданные источника архива указывают за пределы каталога источника.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_METADATA_EXCEEDS_ENTRY_LIMIT":
    "Метаданные источника архива превышают ограничение числа элементов.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_METADATA_MUST_IDENTIFY_REGULAR_FILE":
    "Метаданные источника архива должны указывать на обычный файл.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_NOT_REGULAR_DIRECTORY":
    "Источник архива не является обычным каталогом.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_SYMLINK_FORBIDDEN":
    "Источник архива не должен быть символической ссылкой.",
  "error.reason.IMPORT_ARCHIVE_STAGING_OUTPUT_DIRECTORY_NOT_EMPTY":
    "Промежуточный выходной каталог архива с выбранным значением не пуст.",
  "error.reason.IMPORT_ARR_SOURCE_WARMUP_RESULT_NOT_FOUND":
    "Результат предварительной подготовки источника arr не найден.",
  "error.reason.IMPORT_CHANGED_WHILE_RETRY_REQUESTED":
    "Импорт изменился во время запроса повторной попытки; обновите его историю.",
  "error.reason.IMPORT_CLEANUP_ATTRIBUTION_MISSING":
    "У записанной операции очистки нет корректной привязки к маршрутизации.",
  "error.reason.IMPORT_CLEANUP_LOCATOR_REUSED_IDENTITY_AMBIGUOUS":
    "Указатель объекта очистки используется повторно или его идентичность неоднозначна.",
  "error.reason.IMPORT_CLEANUP_STATE_INVALID":
    "Записанное состояние очистки недопустимо.",
  "error.reason.IMPORT_CLEANUP_TARGET_IS_OUTPUT_ROOT":
    "Удаление корневого выходного каталога клиента загрузки с выбранным значением запрещено.",
  "error.reason.IMPORT_CLEANUP_TARGET_OVERLAPS_LIBRARY_ROOT":
    "Удаление загруженных клиентом данных с выбранным значением запрещено, поскольку они пересекаются с настроенной корневой папкой библиотеки.",
  "error.reason.IMPORT_CLIENT_DOES_NOT_REPORT_COMPLETED_DOWNLOAD":
    "Клиент не сообщает о завершённой загрузке; повторная попытка импорта не может перезапустить неудавшуюся или активную загрузку.",
  "error.reason.IMPORT_CLIENT_JOB_ID_NOW_BELONGS_DIFFERENT_DOWNLOAD":
    "ID задания клиента теперь относится к другой загрузке; используйте ручной импорт для сохранённых файлов.",
  "error.reason.IMPORT_CLIENT_REPORTED_SUBTITLE_NOT_REGULAR_FILE":
    "Указанный клиентом файл субтитров не является обычным файлом.",
  "error.reason.IMPORT_CLIENT_REPORTED_SUBTITLE_REPARSE_POINT":
    "Указанный клиентом файл субтитров является точкой повторной обработки.",
  "error.reason.IMPORT_CLIENT_RETURNED_DIFFERENT_DOWNLOAD_IDENTITY":
    "Клиент вернул другие идентификационные данные загрузки; обновите данные и повторите попытку.",
  "error.reason.IMPORT_COMPLETED_DOWNLOAD_NOT_OBSERVED_YET":
    "Завершённая загрузка ещё не обнаружена; повторно освободите её после обнаружения.",
  "error.reason.IMPORT_COMPLETED_SOURCE_NO_TRUSTWORTHY_DOWNLOAD_IDENTITY":
    "У завершённого источника нет достоверных идентификационных данных загрузки; используйте ручной импорт.",
  "error.reason.IMPORT_DESTINATION_OCCUPIED":
    "Восстановление старой резервной копии обновления с выбранным значением запрещено, поскольку место назначения занято.",
  "error.reason.IMPORT_DISC_IMPORT_TARGETS_INVALID":
    "Цели импорта диска должны в точности соответствовать явно заданным сопоставлениям эпизодов.",
  "error.reason.IMPORT_DISC_SELECTION_CANNOT_INCLUDE_SEPARATE_EPISODE_SERIES_MOVIE_TARGET":
    "Выбор содержимого диска не может включать отдельную цель эпизода или фильма сериала.",
  "error.reason.IMPORT_DISC_SELECTION_MAPPING_LIMIT_EXCEEDED_INVALID":
    "Недопустимый выбор содержимого диска или превышено ограничение числа сопоставлений.",
  "error.reason.IMPORT_DISC_TITLES_REQUIRE_DISTINCT_EXPLICIT_SINGLE_EPISODE_MAPPINGS":
    "Тайтлы диска требуют отдельных явно заданных сопоставлений с одиночными эпизодами.",
  "error.reason.IMPORT_DISK_FULL":
    "Недостаточно места на диске для импорта файла.",
  "error.reason.IMPORT_DOWNLOAD_ALREADY_BEING_IMPORTED":
    "Эта загрузка уже импортируется.",
  "error.reason.IMPORT_DOWNLOAD_ALREADY_SETTLED":
    "Обработка загрузки уже завершена; обновите её историю.",
  "error.reason.IMPORT_DOWNLOAD_AWAITING_IMPORT_RETRY_RECONCILIATION":
    "Эта загрузка ожидает сверки состояния повторного импорта.",
  "error.reason.IMPORT_DOWNLOAD_BUSY_IMPORT_CHANGED":
    "Загрузка занята или импорт изменился; обновите данные и повторите попытку.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_COMPLETED_DOWNLOAD_NOT_FOUND":
    "Завершённая загрузка клиента загрузки с выбранным значением недоступна для очистки данных.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_CONFIGURATION_NO_LONGER_MATCHES_SOURCE":
    "Настройки клиента загрузки больше не соответствуют этому источнику; используйте ручной импорт.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_NOT_FOUND": "Клиент загрузки не найден.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PATH_MAPPING_UNUSABLE_LOCAL_ROOT":
    "В сопоставлении путей клиента загрузки указан непригодный локальный корневой каталог.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_CHECKPOINT_NOT_DIRECTORY":
    "Контрольная точка очистки данных клиента загрузки с выбранным значением не является каталогом.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_REQUIRES_CONFIGURED_CLIENT_ID":
    "Для очистки данных клиента загрузки требуется настроенный идентификатор клиента.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_INVALID":
    "Путь к данным клиента загрузки должен быть абсолютным.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_NOT_FILE_DIRECTORY_SYMLINK":
    "Путь к данным клиента загрузки с выбранным значением не является файлом, каталогом или символической ссылкой.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_OUTSIDE_OUTPUT_ROOT":
    "Путь к данным клиента загрузки с выбранным значением находится за пределами выходного корневого каталога с выбранным значением.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_TRAVERSES_SYMLINK":
    "Путь к данным клиента загрузки с выбранным значением проходит через символическую ссылку с выбранным значением.",
  "error.reason.IMPORT_DOWNLOAD_FAILED":
    "Загрузка завершилась с ошибкой; повторите загрузку перед импортом.",
  "error.reason.IMPORT_DOWNLOAD_HOLDS_MORE_ARCHIVE_SETS":
    "В загрузке больше наборов архивов, чем допускает выбранное значение.",
  "error.reason.IMPORT_DOWNLOAD_NO_STABLE_IDENTITY_YET":
    "У этой загрузки пока нет устойчивых идентификационных данных; её источники остаются удержанными.",
  "error.reason.IMPORT_DOWNLOAD_NO_TITLE_AUTHORIZE_RELEASE_AGAINST":
    "У импорта этой загрузки нет тайтла, по которому можно разрешить освобождение её источников.",
  "error.reason.IMPORT_DOWNLOAD_PAYLOAD_DIRECTORY_NO_LONGER_REAL_DIRECTORY":
    "Каталог данных загрузки с выбранным значением больше не является реальным каталогом.",
  "error.reason.IMPORT_DOWNLOAD_PAYLOAD_DIRECTORY_TOO_MANY_ENTRIES_INVENTORY_SAFELY":
    "В каталоге данных загрузки с выбранным значением слишком много элементов для безопасной инвентаризации.",
  "error.reason.IMPORT_DOWNLOAD_PAYLOAD_INVENTORY_ENTRY_NOT_SAFE_RELATIVE_PATH":
    "Элемент описи данных загрузки с выбранным значением не является безопасным относительным путём.",
  "error.reason.IMPORT_DOWNLOAD_STATE_UNAVAILABLE":
    "Не удалось определить текущее состояние загрузки в клиенте; повторите попытку, когда его история станет доступна.",
  "error.reason.IMPORT_DOWNLOAD_STILL_BEING_IMPORTED":
    "Эта загрузка ещё импортируется; освободите её источники после завершения импорта.",
  "error.reason.IMPORT_DO_NOT_ACCESS_ANY_LIBRARIES":
    "У вас нет доступа ни к одной библиотеке.",
  "error.reason.IMPORT_EPISODE_LOOKUP_FAILED": "Не удалось загрузить эпизод.",
  "error.reason.IMPORT_EPISODE_NOT_FOUND": "Эпизод не найден.",
  "error.reason.IMPORT_EPISODIC_DISCS_REQUIRE_PLAYBACK_TITLE_MAPPINGS":
    "Для дисков с эпизодами требуется сопоставление воспроизводимых тайтлов; диски с фильмами нельзя сопоставлять с эпизодами.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_DEDUPKEY_DUPLICATED":
    "В выбранном значении повторяется dedupKey настройки внешнего импорта.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_INSTANCEID_DUPLICATED":
    "Повторяется instanceId настройки внешнего импорта.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_CANNOT_EMPTY":
    "Черновик секретных данных настройки внешнего импорта не может быть пустым; вместо этого удалите черновик.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_OUT_OF_RANGE":
    "Размер черновика секретных данных настройки внешнего импорта с выбранным значением не должен превышать выбранное число байтов.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_REQUIRED":
    "Черновик секретных данных настройки внешнего импорта с выбранным значением не может быть пустым.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST":
    "Черновик секретных данных настройки внешнего импорта поддерживает не более выбранного количества элементов выбранного типа.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST_INSTANCE_API_KEYS":
    "Черновик секретных данных настройки внешнего импорта поддерживает не более выбранного числа API-ключей экземпляров.",
  "error.reason.IMPORT_EXTRACTED_ARCHIVE_WORKSPACE_UNAVAILABLE":
    "Рабочий каталог распакованного архива недоступен.",
  "error.reason.IMPORT_FILE_NOT_FOUND": "Исходный файл не найден.",
  "error.reason.IMPORT_FILE_PAYLOAD_CHANGED_SINCE_IMPORT":
    "Данные файла изменились после импорта; источник и запись в клиенте сохранены.",
  "error.reason.IMPORT_FILE_PAYLOAD_CLEANUP_REQUIRES_DURABLE_JOB_OWNERSHIP":
    "Для очистки данных файла требуется подтверждённое сохранённое владение со стороны задания.",
  "error.reason.IMPORT_FILE_PAYLOAD_OWNERSHIP_UNVERIFIED":
    "Принадлежность данных файла не подтверждена; источник и запись в клиенте сохранены.",
  "error.reason.IMPORT_GRAB_DOWNLOAD_CAME_COULD_NOT_RESOLVED":
    "Не удалось определить запрос на скачивание, из которого появилась эта загрузка; повторите освобождение, когда он станет доступен.",
  "error.reason.IMPORT_IMPORTED_VIDEO_NO_FILENAME_STEM":
    "У импортированного видео нет имени файла без расширения.",
  "error.reason.IMPORT_IMPORTED_VIDEO_NO_PARENT_DIRECTORY":
    "У импортированного видео нет родительского каталога.",
  "error.reason.IMPORT_INSUFFICIENT_DISK_ALLOWANCE_SCENE_SUBTITLES":
    "Недостаточно выделенного дискового пространства для субтитров сцены; рабочий каталог сохранён.",
  "error.reason.IMPORT_IO_FAILED":
    "Операция с файловой системой завершилась с ошибкой.",
  "error.reason.IMPORT_LIBRARY_ACCESS_DENIED":
    "У вас нет доступа к этой библиотеке.",
  "error.reason.IMPORT_MANUAL_IMPORT_CANDIDATE_IDS_INVALID":
    "Идентификаторы кандидатов для ручного импорта должны быть уникальными.",
  "error.reason.IMPORT_MANUAL_IMPORT_CANDIDATE_ID_REQUIRED":
    "Требуется идентификатор кандидата для ручного импорта.",
  "error.reason.IMPORT_MANUAL_IMPORT_CANDIDATE_REQUIRED":
    "Требуется хотя бы один кандидат для ручного импорта.",
  "error.reason.IMPORT_MANUAL_IMPORT_EPISODE_TARGET_DOES_NOT_BELONG_TITLE":
    "Целевой эпизод ручного импорта с выбранным значением не принадлежит тайтлу с выбранным значением.",
  "error.reason.IMPORT_MANUAL_IMPORT_EPISODE_TARGET_UNAVAILABLE":
    "Целевой эпизод ручного импорта недоступен.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_INVALID":
    "У файла ручного импорта должно быть имя.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_OUTSIDE_TRUSTED_SOURCE_ROOT":
    "Файл ручного импорта находится за пределами доверенного корневого каталога источника.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_PATH_OUTSIDE_TRUSTED_SOURCE_ROOT":
    "Путь к файлу ручного импорта находится за пределами доверенного корневого каталога источника.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_UNAVAILABLE":
    "Файл ручного импорта недоступен.",
  "error.reason.IMPORT_MANUAL_IMPORT_MAPPING_CANNOT_INCLUDE_BOTH_EPISODE_ID_SERIES_MOVIE_LINK_ID":
    "Сопоставление ручного импорта не может одновременно содержать episode_id и series_movie_link_id.",
  "error.reason.IMPORT_MANUAL_IMPORT_MAPPING_REQUIRES_EPISODE_ID_SERIES_MOVIE_LINK_ID":
    "Для сопоставления ручного импорта требуется episode_id или series_movie_link_id.",
  "error.reason.IMPORT_MANUAL_IMPORT_PATH_UNAVAILABLE":
    "Путь ручного импорта недоступен.",
  "error.reason.IMPORT_MANUAL_IMPORT_REQUEST_ACTOR_NO_LONGER_EXISTS":
    "Инициатор запроса ручного импорта больше не существует.",
  "error.reason.IMPORT_MANUAL_IMPORT_REQUEST_MISSING_ACTOR":
    "В запросе ручного импорта не указан инициатор.",
  "error.reason.IMPORT_MANUAL_IMPORT_SERIES_MOVIE_TARGET_DOES_NOT_BELONG_TITLE":
    "Целевой фильм сериала для ручного импорта с выбранным значением не принадлежит тайтлу с выбранным значением.",
  "error.reason.IMPORT_MANUAL_IMPORT_SERIES_MOVIE_TARGET_UNAVAILABLE":
    "Целевой фильм сериала для ручного импорта недоступен.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_CONFLICTING_CANONICAL_DOWNLOAD_IDENTITY":
    "Канонические идентификационные данные загрузки источника ручного импорта конфликтуют с существующими.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_NOT_FILE":
    "Источник ручного импорта не является файлом.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_ROOT_NOT_FILE_DIRECTORY":
    "Корень источника ручного импорта не является файлом или каталогом.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_ROOT_REQUIRED":
    "Требуется корень источника ручного импорта.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_ROOT_UNAVAILABLE":
    "Корень источника ручного импорта недоступен.",
  "error.reason.IMPORT_MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD":
    "Тайтл ручного импорта не соответствует запросу Scryer, по которому была получена эта загрузка.",
  "error.reason.IMPORT_MANUAL_TITLE_OUTSIDE_DURABLE_SCRYER_SUBMISSION_TITLE":
    "Выбранный вручную тайтл с выбранным значением не входит в тайтл сохранённого запроса Scryer с выбранным значением.",
  "error.reason.IMPORT_MAPPED_DISC_EPISODE_NO_LONGER_BELONGS_IMPORT_TITLE":
    "Сопоставленный эпизод диска больше не принадлежит тайтлу импорта.",
  "error.reason.IMPORT_MEDIA_FILE_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED":
    "Медиафайл изменился до публикации метаданных импорта.",
  "error.reason.IMPORT_MEDIA_FILE_NOT_FOUND": "Медиафайл не найден.",
  "error.reason.IMPORT_MEDIA_ROOT_REQUIRED":
    "Невозможно безопасно заменить выбранный элемент улучшенной версией: не удалось определить настроенный корневой каталог медиа.",
  "error.reason.IMPORT_MULTIPLE_SCENE_SUBTITLES_MAP_SAME_DESTINATION":
    "Несколько субтитров сцены сопоставлены с одним путём назначения; источники сохранены.",
  "error.reason.IMPORT_NEWER_IMPORT_ATTEMPT_OWNS_DOWNLOAD":
    "Этой загрузкой владеет более новая попытка импорта; обновите её историю.",
  "error.reason.IMPORT_NOT_HOLDING_SOURCES":
    "Импорт с выбранным значением не удерживает свои источники.",
  "error.reason.IMPORT_NO_COMPLETED_IMPORT_ATTEMPT_UNAVAILABLE":
    "Нет завершённой попытки импорта для повтора; используйте ручной импорт.",
  "error.reason.IMPORT_NZBGET_CLEANUP_IDENTITY_CHANGED":
    "Идентификационные данные очистки NZBGet изменились.",
  "error.reason.IMPORT_NZBGET_HISTORY_REMOVAL_WOULD_DELETE_RETAINED_UNVERIFIED_DATA":
    "Удаление истории NZBGet приведёт к удалению сохранённых или непроверенных данных.",
  "error.reason.IMPORT_NZBGET_NATIVE_DELETION_PARENT_UNAVAILABLE":
    "Родительский каталог для встроенного удаления NZBGet недоступен.",
  "error.reason.IMPORT_NZBGET_NATIVE_DELETION_PATH_UNVERIFIED":
    "Путь для встроенного удаления NZBGet не подтверждён.",
  "error.reason.IMPORT_NZBGET_TERMINAL_CLEANUP_METADATA_NOT_FOUND":
    "Метаданные окончательной очистки NZBGet недоступны: не найдены.",
  "error.reason.IMPORT_PATH_OUTSIDE_LIBRARY_ROOT":
    "Защитная проверка замены по тому же пути отклонена: старый файл с выбранным значением находится за пределами корневого каталога медиа с выбранным значением.",
  "error.reason.IMPORT_PAYLOAD_DIRECTORY_OWNERSHIP_UNVERIFIED":
    "Принадлежность каталога данных не подтверждена; удаление общего каталога запрещено.",
  "error.reason.IMPORT_PAYLOAD_OVERLAPS_RECYCLE_ROOT":
    "Данные пересекаются с корневым каталогом корзины.",
  "error.reason.IMPORT_PAYLOAD_PATH_UNSAFE":
    "Путь к данным клиента загрузки не должен быть корнем файловой системы или содержать переходы за пределы каталога.",
  "error.reason.IMPORT_PERMISSION_DENIED":
    "Файловая система отказала в доступе при импорте.",
  "error.reason.IMPORT_PROWLARR_WARMUP_RESULT_NOT_FOUND":
    "Результат предварительного запроса Prowlarr не найден.",
  "error.reason.IMPORT_RECORD_CONFLICTS_COMPLETED_SOURCE_IDENTITY":
    "Запись импорта конфликтует с идентификационными данными завершённого источника; используйте ручной импорт.",
  "error.reason.IMPORT_RECYCLE_BIN_CONFIG_REQUIRED":
    "Замена выбранного элемента улучшенной версией запрещена: не удалось определить настройки корзины для корневого каталога старого файла с выбранным значением.",
  "error.reason.IMPORT_REGULAR_FILE_REQUIRED":
    "Удаление старого файла с выбранным значением при замене улучшенной версией запрещено: это не обычный файл.",
  "error.reason.IMPORT_RELEASING_THESE_SOURCES_ALSO_RELEASES_IMPORT_TITLE_MAY_NOT_RESOLVE_IMPORTS":
    "Освобождение этих источников также освободит импорт тайтла, для которого у вас может не быть права разрешать проблемы импорта.",
  "error.reason.IMPORT_RESOURCE_NOT_FOUND": "Импорт не найден.",
  "error.reason.IMPORT_RETRY_ALREADY_RUNNING_AWAITING_RECONCILIATION":
    "Эта повторная попытка импорта уже выполняется или ожидает сверки состояния.",
  "error.reason.IMPORT_RETRY_IMPORT_RECORD_NOT_FOUND":
    "Запись повторного импорта не найдена.",
  "error.reason.IMPORT_SAVED_DISC_SELECTION_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED":
    "Сохранённый выбор содержимого диска изменился до публикации метаданных импорта.",
  "error.reason.IMPORT_SCENE_SUBTITLE_AGGREGATE_SIZE_LIMIT_EXCEEDED":
    "Превышен предельный суммарный размер субтитров сцены; рабочий каталог сохранён.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DELIVERY_CONTEXT_UNAVAILABLE":
    "Контекст доставки субтитров сцены недоступен.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DESTINATION_ALREADY_EXISTS_DIFFERENT_CONTENTS":
    "По пути назначения субтитров сцены уже есть файл с другим содержимым; источник сохранён.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DIRECTORY_ENTRY_LIMIT_EXCEEDED":
    "Превышен лимит элементов каталога субтитров сцены.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DIRECTORY_NESTING_LIMIT_EXCEEDED":
    "Превышен лимит вложенности каталогов субтитров сцены.",
  "error.reason.IMPORT_SCENE_SUBTITLE_FILE_COUNT_LIMIT_EXCEEDED":
    "Превышен лимит числа файлов субтитров сцены; рабочий каталог сохранён.",
  "error.reason.IMPORT_SCENE_SUBTITLE_HANDLE_LIES_OUTSIDE_OWNED_WORKSPACE_REPARSE_POINT":
    "Дескриптор субтитров сцены указывает за пределы принадлежащего им рабочего каталога или является точкой повторной обработки.",
  "error.reason.IMPORT_SCENE_SUBTITLE_HANDLE_PATH_INVALID_UTF_16":
    "Путь дескриптора субтитров сцены содержит недопустимую последовательность UTF-16.",
  "error.reason.IMPORT_SCENE_SUBTITLE_LIES_OUTSIDE_OWNED_WORKSPACE":
    "Субтитры сцены находятся за пределами принадлежащего им рабочего каталога.",
  "error.reason.IMPORT_SCENE_SUBTITLE_NOT_BOUNDED_REGULAR_FILE":
    "Субтитры сцены не являются обычным файлом с ограниченным размером.",
  "error.reason.IMPORT_SCENE_SUBTITLE_NOT_REGULAR_FILE":
    "Субтитры сцены не являются обычным файлом.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_COMPONENT_INVALID":
    "Недопустимый компонент пути к субтитрам сцены.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_CONTAINS_SYMLINK":
    "Путь к субтитрам сцены содержит символическую ссылку.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_INVALID":
    "Недопустимый путь к субтитрам сцены.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_NO_FILENAME":
    "В пути к субтитрам сцены нет имени файла.",
  "error.reason.IMPORT_SCENE_SUBTITLE_SIZE_LIMIT_EXCEEDED":
    "Превышен предельный размер субтитров сцены.",
  "error.reason.IMPORT_SCENE_SUBTITLE_STAGING_REQUIRES_OWNED_ARCHIVE_WORKSPACE":
    "Для подготовки субтитров сцены требуется принадлежащий заданию рабочий каталог архива.",
  "error.reason.IMPORT_SCENE_SUBTITLE_STAGING_WORKSPACE_UNAVAILABLE":
    "Рабочий каталог подготовки субтитров сцены недоступен.",
  "error.reason.IMPORT_SCENE_SUBTITLE_WORKSPACE_IDENTITY_CHANGED":
    "Идентификационные данные рабочего каталога субтитров сцены изменились.",
  "error.reason.IMPORT_SCHEDULED_SCRIPTS_REQUIRE_SCHEDULE":
    "Для скриптов, запускаемых по расписанию, требуется расписание.",
  "error.reason.IMPORT_SCRIPT_NOT_FOUND": "Скрипт не найден.",
  "error.reason.IMPORT_SCRIPT_S_TRIGGER_CANNOT_CHANGED_AFTER_CREATED":
    "Триггер скрипта нельзя изменить после его создания.",
  "error.reason.IMPORT_SOURCE_JOB_FAILED":
    "Загрузка источника завершилась с ошибкой.",
  "error.reason.IMPORT_SOURCE_UNAVAILABLE":
    "Источник ручного импорта недоступен.",
  "error.reason.IMPORT_STATUS_ONLY_FAILED_SKIPPED_IMPORTS_RETRIED":
    "Импорт с выбранным значением имеет выбранный статус; повторить можно только импорты с ошибкой или пропущенные импорты.",
  "error.reason.IMPORT_SYMLINK_REFUSED":
    "Удаление старого файла с выбранным значением при замене улучшенной версией запрещено: это символическая ссылка.",
  "error.reason.IMPORT_UNAVAILABLE": "Этот импорт больше нельзя отменить.",
  "error.reason.IMPORT_UNSAFE_ARCHIVE_ENTRY_PATH":
    "Небезопасный путь элемента архива.",
  "error.reason.IMPORT_UPGRADE_RECYCLE_BIN_PATH_UNSAFE":
    "Замена улучшенной версией запрещена: путь к корзине небезопасен.",
  "error.reason.INTEGRATION_ACTION_PERMISSION_DENIED":
    "У вас нет разрешения на это действие.",
  "error.reason.INTEGRATION_API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY":
    "Похоже, вместо API-ключа указана замаскированная заглушка — введите настоящий ключ.",
  "error.reason.INTEGRATION_ASSIGN_INDEXER_TEST_HTTP_3_CONNECT_THROUGH_DESTINATION":
    "Назначьте индексатор для проверки HTTP/3 CONNECT через его адрес назначения.",
  "error.reason.INTEGRATION_BASE_URL_CREDENTIALS_FORBIDDEN":
    "Базовый URL не должен содержать встроенные учётные данные.",
  "error.reason.INTEGRATION_BASE_URL_HOST_REQUIRED":
    "Базовый URL должен содержать хост.",
  "error.reason.INTEGRATION_BASE_URL_INVALID":
    "Базовый URL должен использовать http или https.",
  "error.reason.INTEGRATION_CHALLENGE_SOLVER_PROXIES_DO_NOT_ACCEPT_USERNAME_PASSWORD":
    "Прокси для решения проверок не принимают имя пользователя или пароль.",
  "error.reason.INTEGRATION_CLIENT_ID_REQUIRED":
    "Требуется идентификатор клиента.",
  "error.reason.INTEGRATION_CLIENT_NAME_REQUIRED":
    "Имя клиента не может быть пустым.",
  "error.reason.INTEGRATION_CLIENT_TYPE_REQUIRED": "Требуется тип клиента.",
  "error.reason.INTEGRATION_DOWNLOAD_ALREADY_BEING_PROCESSED":
    "Эта загрузка уже обрабатывается.",
  "error.reason.INTEGRATION_DOWNLOAD_ALREADY_RESOLVED":
    "Загрузка уже обработана с выбранным результатом; игнорировать нечего.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_CONFIG_INVALID":
    "Недопустимые настройки клиента загрузки.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND":
    "Настройки клиента загрузки не найдены.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_ALL_PROTOCOL_FAMILIES_REQUIRED_INDEXER":
    "Клиент загрузки с выбранным значением не поддерживает все семейства протоколов, необходимые индексатору с выбранным значением.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_PASSWORD_RETRIES":
    "Этот клиент загрузки не поддерживает повторные попытки с паролем.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_FIELD_REQUIRED":
    "Нужно указать хотя бы одно поле клиента загрузки.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_ITEM_ID_REQUIRED":
    "Требуется идентификатор элемента клиента загрузки.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_NAME_REQUIRED":
    "Требуется имя клиента загрузки.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_TYPE_UNSUPPORTED":
    "Неподдерживаемый тип клиента загрузки: выбранный элемент.",
  "error.reason.INTEGRATION_DOWNLOAD_NOT_STATE_IMPORTED_HAND_RIGHT_NOW":
    "Текущее состояние этой загрузки не позволяет импортировать её вручную.",
  "error.reason.INTEGRATION_DOWNLOAD_SUBMISSION_NOT_FOUND":
    "Запрос на загрузку не найден.",
  "error.reason.INTEGRATION_DOWNLOAD_UNAVAILABLE":
    "Загрузка больше недоступна для ручного импорта.",
  "error.reason.INTEGRATION_ENABLE_PROXY_BEFORE_TESTING_OUTBOUND_ROUTE":
    "Включите этот прокси перед проверкой исходящего маршрута.",
  "error.reason.INTEGRATION_FIELD_REQUIRED":
    "Требуется указать запрошенное значение.",
  "error.reason.INTEGRATION_HELD_SOURCES_COULD_NOT_SETTLED":
    "Не удалось завершить обработку удерживаемых источников.",
  "error.reason.INTEGRATION_HTTP_3_CONNECT_ENDPOINT_INVALID":
    "Адрес HTTP/3 CONNECT должен иметь вид https://host[:port] без пути, строки запроса и фрагмента.",
  "error.reason.INTEGRATION_HTTP_3_CONNECT_REQUIRES_HTTPS_ENDPOINT":
    "Для HTTP/3 CONNECT требуется адрес с протоколом https.",
  "error.reason.INTEGRATION_HTTP_PROXY_BASE_URL_INVALID":
    "Базовый URL HTTP-прокси должен использовать http или https.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_ID_REQUIRED":
    "Требуется идентификатор настроек индексатора.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_INVALID":
    "Недопустимые настройки индексатора.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_JSON_INVALID":
    "Поле config_json индексатора должно быть объектом JSON.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_NOT_FOUND":
    "Настройки индексатора не найдены.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_TEST_DID_NOT_RETURN_VALID_NEWZNAB_CAPS_DOCUMENT":
    "Проверка подключения к индексатору не вернула корректный документ Newznab caps.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_TEST_FAILED":
    "Проверка подключения к индексатору завершилась с ошибкой.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_UNREACHABLE":
    "Не удалось связаться с индексатором.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_URL_REQUIRED":
    "Требуется URL подключения к индексатору.",
  "error.reason.INTEGRATION_INDEXER_DOES_NOT_SUPPORT_TORRENTS_SO_SEEDING_PROFILE_CANNOT_ASSIGNED":
    "Индексатор с выбранным значением не поддерживает торренты, поэтому ему нельзя назначить профиль сидирования с выбранным значением.",
  "error.reason.INTEGRATION_INDEXER_FIELD_REQUIRED":
    "Нужно указать хотя бы одно поле индексатора.",
  "error.reason.INTEGRATION_INDEXER_ID_REQUIRED":
    "Требуется идентификатор индексатора.",
  "error.reason.INTEGRATION_INDEXER_NAME_REQUIRED":
    "Имя индексатора не может быть пустым.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_DECLARES_MULTIPLE_CONNECTION_URL_CONFIG_FIELDS":
    "Провайдер индексатора объявляет несколько полей настроек connection_url.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_DOES_NOT_DECLARE_SUPPORTED_PROTOCOL_FAMILY":
    "Провайдер индексатора с выбранным значением не объявляет поддерживаемое семейство протоколов.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_TYPE_UNSUPPORTED":
    "Неподдерживаемый тип провайдера индексатора: выбранный элемент.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_UNAVAILABLE":
    "Провайдер индексатора недоступен.",
  "error.reason.INTEGRATION_INDEXER_REQUEST_REJECTED":
    "Индексатор отклонил запрос.",
  "error.reason.INTEGRATION_INDEXER_SETTINGS_CHANGED_DURING_VALIDATION":
    "Настройки индексатора изменились во время проверки; обновите данные и повторите проверку.",
  "error.reason.INTEGRATION_LIBRARY_ACCESS_DENIED":
    "У вас нет доступа к этой библиотеке.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXERS_CANNOT_SYNCED_DIRECTLY":
    "Управляемые дочерние индексаторы нельзя синхронизировать напрямую.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC":
    "Управляемые дочерние индексаторы контролируются синхронизацией родительского индексатора.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC_CANNOT_EDITED_DIRECTLY":
    "Управляемые дочерние индексаторы контролируются синхронизацией родительского индексатора и не могут редактироваться напрямую.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXER_REQUIRES_CHILD_KEY":
    "Для управляемого дочернего индексатора требуется дочерний ключ.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXER_REQUIRES_PARENT":
    "Для управляемого дочернего индексатора требуется родительский индексатор.",
  "error.reason.INTEGRATION_MANAGED_CHILD_PLAN_CONTAINS_DUPLICATE_CHILD_KEY":
    "План управляемых дочерних индексаторов содержит повторяющийся child_key с выбранным значением.",
  "error.reason.INTEGRATION_MANAGED_CHILD_PLAN_ENTRIES_REQUIRE_CHILD_KEY":
    "Для элементов плана управляемых дочерних индексаторов требуется child_key.",
  "error.reason.INTEGRATION_MANAGED_CHILD_REQUIRES_NAME":
    "Для управляемого дочернего элемента с выбранным значением требуется имя.",
  "error.reason.INTEGRATION_MANAGED_CHILD_REQUIRES_PROVIDER_TYPE":
    "Для управляемого дочернего элемента с выбранным значением требуется provider_type.",
  "error.reason.INTEGRATION_MANAGED_CHILD_ROUTING_CONTAINS_DUPLICATE_SCOPE":
    "Маршрутизация управляемых дочерних элементов содержит повторяющуюся область с выбранным значением.",
  "error.reason.INTEGRATION_MANAGED_CHILD_ROUTING_SCOPE_UNSUPPORTED":
    "Область маршрутизации управляемых дочерних элементов с выбранным значением не поддерживается.",
  "error.reason.INTEGRATION_MANAGED_INDEXERS_CANNOT_USE_PROXY":
    "Управляемые индексаторы не могут использовать прокси; решением проверок занимается управляющее приложение.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_CANDIDATE_UNAVAILABLE":
    "Кандидат для ручного импорта недоступен.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_FILES_UNAVAILABLE":
    "Файлы ручного импорта больше недоступны.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_SELECTION_CHANGED":
    "Выбор для ручного импорта изменился; откройте окно импорта заново.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_SELECTION_TITLE_UNAVAILABLE":
    "Тайтл, выбранный для ручного импорта, недоступен.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_SELECTION_UNAVAILABLE":
    "Выбор для ручного импорта недоступен; откройте окно импорта заново.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD":
    "Тайтл ручного импорта не соответствует запросу Scryer, по которому была получена эта загрузка.",
  "error.reason.INTEGRATION_MAPPED_DOWNLOAD_CLIENT_NO_LONGER_EXISTS":
    "Сопоставленный клиент загрузки с выбранным значением больше не существует.",
  "error.reason.INTEGRATION_MAX_QUERIES_PER_MINUTE_OUT_OF_RANGE":
    "Максимальное число запросов в минуту должно быть не меньше 1; оставьте поле пустым, чтобы не ограничивать число запросов.",
  "error.reason.INTEGRATION_MAX_QUERIES_PER_MINUTE_SET_EACH_MANAGED_INDEXER_NOT_INDEXER_SYNCS_THEM":
    "Максимальное число запросов в минуту задаётся для каждого управляемого индексатора, а не для индексатора, который их синхронизирует.",
  "error.reason.INTEGRATION_MINIMUM_SEEDERS_FLOOR_CANNOT_NEGATIVE":
    "Нижний предел минимального числа сидов не может быть отрицательным.",
  "error.reason.INTEGRATION_NOT_DNS_SERVER_ADDRESS":
    "`выбранное значение` не является адресом DNS-сервера; строка `DNS` принимает IP-адреса.",
  "error.reason.INTEGRATION_NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE":
    "Для выбранного типа провайдера нет доступного клиента управления индексаторами.",
  "error.reason.INTEGRATION_NO_INDEXER_PROVIDER_AVAILABLE_PROVIDER_TYPE":
    "Для выбранного типа провайдера нет доступного провайдера индексаторов.",
  "error.reason.INTEGRATION_ONLY_SSH_TUNNELS_TAKE_PRIVATE_KEY_PASSPHRASE":
    "Парольная фраза закрытого ключа используется только в SSH-туннелях.",
  "error.reason.INTEGRATION_ONLY_SSH_TUNNEL_PROXIES_PIN_HOST_KEY":
    "Закрепление ключа хоста доступно только для прокси через SSH-туннель.",
  "error.reason.INTEGRATION_ONLY_TUNNELS_TAKE_PRIVATE_KEY":
    "Закрытый ключ используется только в туннелях.",
  "error.reason.INTEGRATION_PASSWORD_RETRY_REQUIRES_EXACT_DOWNLOAD_CLIENT":
    "Для повторной попытки с паролем требуется точно указанный клиент загрузки.",
  "error.reason.INTEGRATION_PLUGIN_RETURNED_INVALID_CONFIG_FIELDS":
    "Плагин вернул недопустимые поля настроек.",
  "error.reason.INTEGRATION_PRIVATE_KEY_INVALID":
    "Закрытый ключ должен быть блоком PEM, начинающимся с -----BEGIN и заканчивающимся на -----.",
  "error.reason.INTEGRATION_PROVIDER_TYPE_DOES_NOT_SUPPORT_MANAGED_CHILD_SYNC":
    "Выбранный тип провайдера не поддерживает синхронизацию управляемых дочерних элементов.",
  "error.reason.INTEGRATION_PROVIDER_TYPE_REQUIRED":
    "Тип провайдера не может быть пустым.",
  "error.reason.INTEGRATION_PROWLARR_INDEXERS_CANNOT_USE_CHALLENGE_SOLVERS":
    "Индексаторы Prowlarr не могут использовать средства решения проверок; обработкой проверок занимается Prowlarr.",
  "error.reason.INTEGRATION_PROWLARR_MANAGEMENT_PARENTS_CANNOT_ASSIGNED_SEEDING_PROFILE":
    "Родительским элементам управления Prowlarr нельзя назначить профиль сидирования.",
  "error.reason.INTEGRATION_PROWLARR_MANAGEMENT_PARENTS_CANNOT_MAPPED_DOWNLOAD_CLIENT":
    "Родительские элементы управления Prowlarr нельзя сопоставить с клиентом загрузки.",
  "error.reason.INTEGRATION_PROXIES_DO_NOT_USE_CHALLENGE_SOLVER_PROTOCOL":
    "Прокси запрошенного типа не используют протокол средства решения проверок.",
  "error.reason.INTEGRATION_PROXY_BASE_URL_HOST_REQUIRED":
    "Базовый URL прокси должен содержать хост.",
  "error.reason.INTEGRATION_PROXY_BASE_URL_INVALID":
    "Базовый URL прокси должен содержать порт.",
  "error.reason.INTEGRATION_PROXY_BASE_URL_REQUIRED":
    "Требуется базовый URL прокси.",
  "error.reason.INTEGRATION_PROXY_CONFIGURATION_NOT_FOUND":
    "Настройки прокси не найдены.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ENABLED_DOWNLOAD_CLIENT_S":
    "Настройки прокси назначены включённым клиентам загрузки в выбранном количестве.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ENABLED_INDEXER_S":
    "Настройки прокси назначены включённым индексаторам в выбранном количестве.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ONE_MORE_DOWNLOAD_CLIENTS":
    "Настройки прокси назначены одному или нескольким клиентам загрузки.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ONE_MORE_INDEXERS":
    "Настройки прокси назначены одному или нескольким индексаторам.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ID_REQUIRED":
    "Требуется идентификатор настроек прокси.",
  "error.reason.INTEGRATION_PROXY_CONFIG_NOT_FOUND":
    "Настройки прокси не найдены.",
  "error.reason.INTEGRATION_PROXY_CREDENTIALS_INVALID":
    "Недопустимые учётные данные прокси.",
  "error.reason.INTEGRATION_PROXY_DISABLED_DOWNLOAD_CLIENT":
    "Прокси отключён для этого клиента загрузки.",
  "error.reason.INTEGRATION_PROXY_DISABLED_INDEXER":
    "Прокси отключён для этого индексатора.",
  "error.reason.INTEGRATION_PROXY_FIELD_REQUIRED":
    "Нужно указать хотя бы одно поле прокси.",
  "error.reason.INTEGRATION_PROXY_NAME_REQUIRED": "Требуется имя прокси.",
  "error.reason.INTEGRATION_PROXY_PASSWORD_REQUIRES_USERNAME":
    "Для указания пароля прокси требуется имя пользователя.",
  "error.reason.INTEGRATION_PROXY_TIMEOUT_OUT_OF_RANGE":
    "Время ожидания прокси должно быть от 1 до выбранного числа секунд.",
  "error.reason.INTEGRATION_PROXY_URL_CREDENTIALS_FORBIDDEN":
    "Базовый URL прокси не должен содержать учётные данные; используйте поля имени пользователя и пароля.",
  "error.reason.INTEGRATION_REMOTE_DNS_APPLIES_ONLY_SOCKS_PROXIES":
    "Удалённое разрешение DNS-имён применимо только к SOCKS-прокси.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_FIELD_REQUIRED":
    "Нужно указать хотя бы одно поле профиля сидирования.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_ID_REQUIRED":
    "Требуется идентификатор профиля сидирования.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_NOT_FOUND":
    "Профиль сидирования не найден.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_STILL_ASSIGNED":
    "Профиль сидирования всё ещё назначен выбранному элементу.",
  "error.reason.INTEGRATION_SOCKS4_PROXIES_DO_NOT_CARRY_CREDENTIALS":
    "Прокси SOCKS4 не передают учётные данные; для прокси с аутентификацией используйте SOCKS5.",
  "error.reason.INTEGRATION_SOCKS4_PROXY_BASE_URL_INVALID":
    "Базовый URL прокси SOCKS4 должен использовать socks4 или socks4a.",
  "error.reason.INTEGRATION_SOCKS5H_SOCKS4A_RESOLVE_NAMES_PROXY":
    "Socks5h:// и socks4a:// разрешают имена на прокси; для локального разрешения используйте socks5:// или socks4://.",
  "error.reason.INTEGRATION_SOCKS5_PROXY_BASE_URL_INVALID":
    "Базовый URL прокси SOCKS5 должен использовать socks5 или socks5h.",
  "error.reason.INTEGRATION_SSH_PRIVATE_KEY_INVALID":
    "Не удалось прочитать закрытый ключ SSH.",
  "error.reason.INTEGRATION_SSH_TUNNELS_DO_NOT_ACCEPT_PASSWORDS":
    "SSH-туннели не принимают пароли; используйте закрытый ключ Ed25519.",
  "error.reason.INTEGRATION_SSH_TUNNELS_REQUIRE_ED25519_PRIVATE_KEY":
    "Для SSH-туннелей требуется закрытый ключ Ed25519.",
  "error.reason.INTEGRATION_SSH_TUNNELS_REQUIRE_USERNAME":
    "Для SSH-туннелей требуется имя пользователя.",
  "error.reason.INTEGRATION_SSH_TUNNEL_BASE_URL_INVALID":
    "Базовый URL SSH-туннеля должен использовать ssh.",
  "error.reason.INTEGRATION_STORED_PROXY_PROTOCOL_DOES_NOT_MATCH_PROVIDER":
    "Сохранённый протокол прокси не соответствует его провайдеру.",
  "error.reason.INTEGRATION_TEST_URL_INVALID":
    "Тестовый URL должен использовать HTTP или HTTPS и не содержать учётных данных.",
  "error.reason.INTEGRATION_TRACKED_DOWNLOAD_BUSY_PROCESSING":
    "Отслеживаемая загрузка с выбранным значением занята обработкой.",
  "error.reason.INTEGRATION_TRACKED_DOWNLOAD_NOT_FOUND":
    "Отслеживаемая загрузка не найдена.",
  "error.reason.INTEGRATION_TUNNEL_MTU_OUT_OF_RANGE":
    "MTU туннеля должен находиться между выбранными нижним и верхним значениями; указанное значение выходит за этот диапазон.",
  "error.reason.INTEGRATION_WIREGUARD_ADDRESS_INVALID":
    "Недопустимый адрес интерфейса WireGuard.",
  "error.reason.INTEGRATION_WIREGUARD_ADDRESS_REQUIRED":
    "Для туннелей WireGuard требуется хотя бы один адрес интерфейса; скопируйте строку `Address` из конфигурации WireGuard (например, `10.6.0.2/32`).",
  "error.reason.INTEGRATION_WIREGUARD_BASE_URL_INVALID":
    "Базовый URL WireGuard должен использовать wireguard.",
  "error.reason.INTEGRATION_WIREGUARD_FIELDS_WRONG_TUNNEL_KIND":
    "Открытый ключ узла, предварительно согласованный ключ, адреса туннеля, DNS-серверы, MTU и keepalive используются только в туннелях WireGuard.",
  "error.reason.INTEGRATION_WIREGUARD_KEYS_INVALID":
    "Недопустимые ключи WireGuard.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_AUTHENTICATE_KEYS_NOT_USERNAME_PASSWORD":
    "Туннели WireGuard проходят аутентификацию по ключам, а не по имени пользователя и паролю.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_NO_HOST_KEY_RESET":
    "У туннелей WireGuard нет ключа хоста, который можно сбросить.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_REQUIRE_PEER_S_PUBLIC_KEY_PEER_SECTION":
    "Для туннелей WireGuard требуется открытый ключ узла из раздела `[Peer]`.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_REQUIRE_PRIVATE_KEY":
    "Для туннелей WireGuard требуется закрытый ключ; выбранное значение.",
  "error.reason.JOB_TASK_FAILED":
    "Запрошенная фоновая задача завершилась с ошибкой.",
  "error.reason.JOB_ACQUISITION_SEARCH_JOBS_INVALID":
    "Задания поиска для получения релизов необходимо запускать через мутацию поиска для получения релизов.",
  "error.reason.JOB_ACTION_PERMISSION_DENIED":
    "У вас нет разрешения на это действие.",
  "error.reason.JOB_ALREADY_RUNNING": "Выбранный элемент уже выполняется.",
  "error.reason.JOB_APPLICATION_UPGRADE_JOBS_INVALID":
    "Задания обновления приложения необходимо запускать через мутацию обновления приложения.",
  "error.reason.JOB_ARTWORK_ENCODING_SHUTTING_DOWN":
    "Кодирование изображений завершает работу.",
  "error.reason.JOB_BACKGROUND_LIBRARY_REFRESH_TEMPORARILY_DISABLED":
    "Фоновое обновление библиотеки временно отключено.",
  "error.reason.JOB_CUSTOM_JOB_RUN_DOES_NOT_NAME_SCRIPT":
    "В запуске пользовательского задания не указан скрипт.",
  "error.reason.JOB_CUSTOM_JOB_RUN_NEEDS_ID_SCHEDULED_SCRIPT_RUN":
    "Для запуска пользовательского задания требуется идентификатор запускаемого скрипта с расписанием.",
  "error.reason.JOB_DISCOVERY_INCREMENTAL_REQUIRES_ACTIVE_GENERATION":
    "Для инкрементального обнаружения требуется активное поколение.",
  "error.reason.JOB_DISCOVERY_INCREMENTAL_REQUIRES_PREVIOUS_FINGERPRINT":
    "Для инкрементального обнаружения требуется предыдущий отпечаток.",
  "error.reason.JOB_EMPTYING_RECYCLE_BIN_ALREADY_RUNNING":
    "Очистка корзины уже выполняется.",
  "error.reason.JOB_FULL_HASH_BACKFILL_SHUTTING_DOWN":
    "Фоновое заполнение полных хешей завершает работу.",
  "error.reason.JOB_LIBRARY_ACCESS_DENIED":
    "У вас нет доступа к этой библиотеке.",
  "error.reason.JOB_LOCATION_OPERATIONS_INVALID":
    "Операции с расположением необходимо запускать через мутацию операции с расположением.",
  "error.reason.JOB_MEDIA_FILE_DELETION_JOBS_INVALID":
    "Задания удаления медиафайлов необходимо запускать через мутацию удаления медиафайлов.",
  "error.reason.JOB_NO_LIBRARIES": "У запрошенного элемента нет библиотек.",
  "error.reason.JOB_ONLY_RUN_CONFIGURED_SCHEDULE":
    "Запрошенный элемент может запускаться только по настроенному расписанию.",
  "error.reason.JOB_PATH_OUTSIDE_LIBRARY_ROOT":
    "Восстановление элемента корзины с выбранным значением запрещено: исходный путь находится за пределами определённых корневых каталогов библиотек.",
  "error.reason.JOB_RECYCLE_BIN_BATCHES_CANNOT_INCLUDE_SAME_ITEM_MORE_ONCE":
    "Пакет операций с корзиной не может включать один элемент более одного раза.",
  "error.reason.JOB_RECYCLE_BIN_BATCHES_LIMITED_ITEMS":
    "Число элементов в пакете операций с корзиной ограничено выбранным значением.",
  "error.reason.JOB_RECYCLE_BIN_ITEM_REQUIRED":
    "Выберите хотя бы один элемент корзины.",
  "error.reason.JOB_RECYCLE_ENTRY_NOT_FOUND": "Элемент корзины не найден.",
  "error.reason.JOB_RECYCLE_OPERATION_ALREADY_RUNNING":
    "Для выбранного элемента уже выполняется операция с корзиной.",
  "error.reason.JOB_RECYCLE_PURGE_JOBS_INVALID":
    "Задания окончательного удаления из корзины необходимо запускать через мутацию корзины.",
  "error.reason.JOB_RECYCLE_RESTORE_ALREADY_RUNNING":
    "Для выбранного элемента уже выполняется восстановление из корзины.",
  "error.reason.JOB_RECYCLE_RESTORE_ALREADY_RUNNING_ONE_SELECTED_DESTINATIONS":
    "Для одного из выбранных путей назначения уже выполняется восстановление из корзины.",
  "error.reason.JOB_RECYCLE_RESTORE_JOBS_INVALID":
    "Задания восстановления из корзины необходимо запускать через мутацию восстановления из корзины.",
  "error.reason.JOB_RECYCLE_RESTORE_PREVIEW_STALE":
    "Предварительный просмотр восстановления из корзины устарел; заново проверьте выбранный способ разрешения конфликта.",
  "error.reason.JOB_REGULAR_FILE_REQUIRED":
    "Замена по пути назначения восстановления с выбранным значением запрещена: это не обычный файл.",
  "error.reason.JOB_REPLACE_RECYCLE_BIN_UNAVAILABLE":
    "Замена выбранного элемента запрещена: корзина недоступна.",
  "error.reason.JOB_RESTORE_DESTINATION_CHANGED_AFTER_CONFIRMATION":
    "Путь назначения восстановления изменился после подтверждения; просмотрите этот элемент и повторите попытку.",
  "error.reason.JOB_SCHEDULED_SCRIPT_ALREADY_RUNNING":
    "Этот скрипт по расписанию уже выполняется.",
  "error.reason.JOB_SCHEDULED_SCRIPT_DISABLED":
    "Скрипт по расписанию с выбранным значением отключён.",
  "error.reason.JOB_SCHEDULED_SCRIPT_NO_LONGER_EXISTS":
    "Скрипт по расписанию с выбранным значением больше не существует.",
  "error.reason.JOB_SCRIPT_NOT_FOUND": "Скрипт не найден.",
  "error.reason.JOB_SCRIPT_NOT_SCHEDULED_SCRIPT":
    "Скрипт с выбранным значением не является скриптом с расписанием.",
  "error.reason.JOB_SELECTED_RECYCLE_ENTRIES_CANNOT_SHARE_SAME_RESTORE_DESTINATION":
    "Выбранные элементы корзины не могут иметь один и тот же путь назначения восстановления.",
  "error.reason.JOB_SELECT_FILES_SEASONS_TITLES_REFRESH_SCORES":
    "Выберите файлы, сезоны или тайтлы для обновления оценок.",
  "error.reason.JOB_SYMLINK_REFUSED":
    "Замена по пути назначения восстановления с выбранным значением запрещена: это символическая ссылка.",
  "error.reason.JOB_TITLE_DELETION_JOBS_INVALID":
    "Задания удаления тайтлов необходимо запускать через мутацию удаления тайтлов.",
  "error.reason.JOB_TITLE_RENAME_JOBS_INVALID":
    "Задания переименования тайтлов необходимо запускать через мутацию переименования.",
  "error.reason.JOB_VIEW_RECYCLE_BIN_PERMISSION_DENIED":
    "У вас нет разрешения на просмотр корзины.",
  "error.reason.LIBRARY_AUTOMATIC_TITLE_DISCOVERY_DEPTH_LIMIT_REACHED":
    "Достигнут предел глубины автоматического обнаружения тайтлов.",
  "error.reason.LIBRARY_BATCH_SIZE_OUT_OF_RANGE":
    "Размер пакета должен быть положительным.",
  "error.reason.LIBRARY_COLLECTION_DOES_NOT_BELONG_TITLE":
    "Коллекция с выбранным значением не принадлежит тайтлу с выбранным значением.",
  "error.reason.LIBRARY_CONFIGURED_ROOT_FOLDER_INVALID":
    "Настроенная корневая папка выбранного типа с выбранным значением недопустима.",
  "error.reason.LIBRARY_DEFAULT_LIBRARIES_CANNOT_DELETED":
    "Библиотеки по умолчанию нельзя удалить.",
  "error.reason.LIBRARY_DEFAULT_LIBRARY_NOT_FOUND":
    "Библиотека по умолчанию не найдена.",
  "error.reason.LIBRARY_DELETE_TARGET_NOT_FILE":
    "Удаление каталога по пути с выбранным значением через операцию удаления отдельного файла запрещено.",
  "error.reason.LIBRARY_DELETE_TARGET_OUTSIDE_ROOTS":
    "Удаление выбранного элемента с выбранным значением запрещено: он находится за пределами настроенных корневых папок.",
  "error.reason.LIBRARY_DELETE_TARGET_PARENT_REQUIRED":
    "Удаление выбранного элемента с выбранным значением запрещено: у него нет родительского каталога.",
  "error.reason.LIBRARY_DESTINATION_FOLDER_CONTAINS_INDEPENDENT_UNRESOLVED_TITLE_BOUNDARY":
    "Папка назначения содержит границу другого тайтла или тайтла с неустановленной принадлежностью.",
  "error.reason.LIBRARY_DESTINATION_OCCUPIED":
    "Восстановление файла из корзины с выбранным значением запрещено: путь назначения занят.",
  "error.reason.LIBRARY_EPISODE_FOLDER_OPTIONAL_GROUP_INVALID":
    "Недопустимая необязательная группа в шаблоне папки эпизода.",
  "error.reason.LIBRARY_EPISODE_FOLDER_PATTERN_INVALID":
    "Недопустимый шаблон папки эпизода.",
  "error.reason.LIBRARY_EPISODE_ID_REQUIRED":
    "Требуется хотя бы один идентификатор эпизода.",
  "error.reason.LIBRARY_EPISODE_SELECTION_REQUIRED":
    "Необходимо выбрать хотя бы один эпизод.",
  "error.reason.LIBRARY_EXTERNAL_SUBTITLE_NOT_FOUND":
    "Внешние субтитры не найдены.",
  "error.reason.LIBRARY_FAILED_ATTACH_PENDING_IMPORT_FILE_SELECTED_MOVIE":
    "Не удалось прикрепить файл ожидающего импорта к выбранному фильму.",
  "error.reason.LIBRARY_FAILED_BIND_PENDING_IMPORT_FILE_SELECTED_EPISODES":
    "Не удалось привязать файл ожидающего импорта к выбранным эпизодам.",
  "error.reason.LIBRARY_FOLDER_NOT_TITLE_BOUNDARY":
    "В папке назначения уже есть папки других тайтлов; это группирующая папка, а не папка тайтла.",
  "error.reason.LIBRARY_FOLDER_OWNERSHIP_CONFLICT":
    "Папки с конфликтами принадлежности нельзя привязать или присвоить.",
  "error.reason.LIBRARY_LINKED_FOLDER_REQUIRES_EXPLICIT_OWNERSHIP_RESOLUTION_BEFORE_SCANNING":
    "Перед сканированием связанной папки требуется явно разрешить вопрос её принадлежности.",
  "error.reason.LIBRARY_MAINTENANCE_DELETION_FILE_CHANGED":
    "Файл для удаления в ходе обслуживания изменился; требуется повторная оценка.",
  "error.reason.LIBRARY_MAINTENANCE_DELETION_FINGERPRINT_CHANGED":
    "Отпечаток удаления в ходе обслуживания изменился.",
  "error.reason.LIBRARY_MAINTENANCE_DELETION_TARGETS_CHANGED":
    "Объекты удаления в ходе обслуживания изменились; требуется повторная оценка.",
  "error.reason.LIBRARY_MEDIA_FILE_NOT_FOUND": "Медиафайл не найден.",
  "error.reason.LIBRARY_MEDIA_IDENTITY_CONFLICTS_PROPOSED_TITLE_FOLDER":
    "Идентификационные данные медиа конфликтуют с предложенной папкой тайтла.",
  "error.reason.LIBRARY_MEDIA_ROOT_REQUIRED":
    "Удаление выбранного элемента запрещено: настроенные корневые каталоги медиа недоступны.",
  "error.reason.LIBRARY_NAME_REQUIRED": "Требуется имя библиотеки.",
  "error.reason.LIBRARY_NO_ROOT_FOLDERS_UNAVAILABLE":
    "Нет доступных настроенных корневых папок выбранного типа для безопасного удаления.",
  "error.reason.LIBRARY_ONE_MORE_SELECTED_EPISODES_DO_NOT_BELONG_TARGET_TITLE":
    "Один или несколько выбранных эпизодов не принадлежат целевому тайтлу.",
  "error.reason.LIBRARY_ONLY_PROVIDER_BACKED_DOWNLOADED_SUBTITLES_BLOCKLISTED":
    "В список блокировки можно добавить только загруженные субтитры, связанные с провайдером.",
  "error.reason.LIBRARY_OWNED_FOLDER_UNAVAILABLE":
    "Принадлежащая тайтлу папка недоступна.",
  "error.reason.LIBRARY_PATH_CANNOT_ESCAPE_ROOT":
    "Путь не может выходить за пределы своего корня.",
  "error.reason.LIBRARY_PATH_COMPONENT_INVALID":
    "Запрошенное значение не должно содержать компоненты . или ...",
  "error.reason.LIBRARY_PATH_INVALID": "Путь должен быть абсолютным.",
  "error.reason.LIBRARY_PATH_NOT_DIRECTORY":
    "Путь библиотеки не является каталогом.",
  "error.reason.LIBRARY_PATH_OUTSIDE_LIBRARY_ROOT":
    "Удаление выбранного элемента запрещено: он находится за пределами настроенных корневых каталогов медиа.",
  "error.reason.LIBRARY_PENDING_IMPORT_ALREADY_BEING_RESOLVED":
    "Ожидающий импорт с выбранным значением уже обрабатывается.",
  "error.reason.LIBRARY_PENDING_IMPORT_DIRECTORY_CONTAINS_NO_FILES_ATTACH":
    "В каталоге ожидающего импорта нет файлов для прикрепления.",
  "error.reason.LIBRARY_PENDING_IMPORT_DOES_NOT_KNOWN_TITLE":
    "Для ожидающего импорта не определён тайтл.",
  "error.reason.LIBRARY_PENDING_IMPORT_FILE_UNAVAILABLE":
    "Файл ожидающего импорта недоступен.",
  "error.reason.LIBRARY_PENDING_IMPORT_ID_REQUIRED":
    "Требуется идентификатор ожидающего импорта.",
  "error.reason.LIBRARY_PENDING_IMPORT_NOT_FOUND":
    "Ожидающий импорт не найден.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_FOLDER_NOT_FILE_SO_CANNOT_BOUND_EPISODES":
    "Путь ожидающего импорта указывает на папку, а не на файл, поэтому его нельзя привязать к эпизодам; устаревшая связь с тайтлом удалена. Обновите список ожидающих импортов и снова прикрепите папку к тайтлу.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_INVALID":
    "Путь ожидающего импорта отсутствует или недопустим.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_NOT_FILE":
    "Путь ожидающего импорта не является файлом.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_NOT_FILE_DIRECTORY":
    "Путь ожидающего импорта не является файлом или каталогом.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_NO_PARENT_FOLDER":
    "У пути ожидающего импорта нет родительской папки.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_UNAVAILABLE":
    "Путь ожидающего импорта недоступен.",
  "error.reason.LIBRARY_PENDING_IMPORT_REQUIRES_EXPLICIT_EPISODE_BINDING":
    "Для ожидающего импорта требуется явная привязка к эпизодам.",
  "error.reason.LIBRARY_RECORDED_MEDIA_PATH_NO_FOLDER":
    "В сохранённом пути к медиа не указана папка.",
  "error.reason.LIBRARY_RECORDED_MEDIA_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER":
    "Сохранённый путь к медиа находится за пределами папки, принадлежащей его тайтлу.",
  "error.reason.LIBRARY_RECORDED_SUBTITLE_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER":
    "Сохранённый путь к субтитрам находится за пределами папки, принадлежащей их тайтлу.",
  "error.reason.LIBRARY_RECYCLE_BIN_PATH_UNSAFE":
    "Перемещение выбранного элемента в корзину запрещено: путь к корзине небезопасен.",
  "error.reason.LIBRARY_RECYCLE_ENTRY_ID_NOT_VALID_OPAQUE_ID":
    "Идентификатор элемента корзины не является допустимым непрозрачным идентификатором.",
  "error.reason.LIBRARY_REGULAR_FILE_REQUIRED":
    "Восстановление файла из корзины с выбранным значением запрещено: это не обычный файл.",
  "error.reason.LIBRARY_RENAMER_DISABLED": "Переименование отключено.",
  "error.reason.LIBRARY_RENAME_ALREADY_RUNNING":
    "Для выбранного элемента уже выполняется переименование.",
  "error.reason.LIBRARY_RENAME_FOLDERS_OVERLAP":
    "Папки выбранных тайтлов пересекаются.",
  "error.reason.LIBRARY_RENAME_PATH_OUTSIDE_ROOT":
    "Предлагаемый путь переименования находится за пределами корневого каталога библиотеки.",
  "error.reason.LIBRARY_RENAME_STALE_PLAN": "План переименования устарел.",
  "error.reason.LIBRARY_REQUESTED_FACET_DOES_NOT_MATCH_TITLE_FACET":
    "Запрошенная категория не соответствует категории тайтла.",
  "error.reason.LIBRARY_RESOURCE_NOT_FOUND": "Библиотека не найдена.",
  "error.reason.LIBRARY_ROOTS_NOT_CONFIGURED":
    "Корневые каталоги библиотеки запрошенного типа не настроены.",
  "error.reason.LIBRARY_ROOT_ALREADY_ASSIGNED":
    "Корневой каталог библиотеки с выбранным значением уже используется выбранным элементом.",
  "error.reason.LIBRARY_ROOT_CANNOT_USED":
    "Корневой каталог библиотеки с выбранным значением нельзя использовать.",
  "error.reason.LIBRARY_ROOT_DUPLICATED":
    "Корневые каталоги внутри библиотеки должны быть уникальными.",
  "error.reason.LIBRARY_ROOT_REQUIRED":
    "Для библиотеки требуется хотя бы одна корневая папка.",
  "error.reason.LIBRARY_SCAN_ALREADY_RUNNING":
    "Сканирование библиотеки уже выполняется.",
  "error.reason.LIBRARY_SCAN_CANCEL_UNAVAILABLE":
    "Сеанс сканирования библиотеки нельзя отменить.",
  "error.reason.LIBRARY_SCAN_FOLDER_BOUNDARY_INVALID":
    "Эта папка не является допустимой границей тайтла для сканирования.",
  "error.reason.LIBRARY_SCAN_SESSION_NOT_FOUND":
    "Сеанс сканирования библиотеки не найден.",
  "error.reason.LIBRARY_SCAN_UNSUPPORTED":
    "Сканирование библиотеки для этой категории не поддерживается.",
  "error.reason.LIBRARY_SCOPED_POLICY_DELETION_CANNOT_REMOVE_DIRECTORIES":
    "Удаление по правилу с ограниченной областью действия не может удалять каталоги.",
  "error.reason.LIBRARY_SELECTED_FOLDER_ORGANIZING_FOLDER_MIXED_TITLE_LAYOUT":
    "Выбранная папка является группирующей или содержит смешанную структуру тайтлов; выберите папку тайтла с его медиафайлами.",
  "error.reason.LIBRARY_SYMLINK_REFUSED":
    "Восстановление файла из корзины с выбранным значением запрещено: это символическая ссылка.",
  "error.reason.LIBRARY_TEMPLATE_BRACES_UNBALANCED":
    "Шаблон запрошенного типа содержит непарную скобку '}}'.",
  "error.reason.LIBRARY_TEMPLATE_IDENTITY_TOKEN_REQUIRED":
    "Шаблон именования должен содержать токен идентификации.",
  "error.reason.LIBRARY_TEMPLATE_OPTIONAL_GROUP_INVALID":
    "Необязательная группа шаблона недопустима.",
  "error.reason.LIBRARY_TEMPLATE_REQUIRED": "Требуется шаблон переименования.",
  "error.reason.LIBRARY_TEMPLATE_TOKEN_UNSUPPORTED":
    "В шаблоне именования используется неподдерживаемый токен.",
  "error.reason.LIBRARY_TITLE_ALREADY_EXISTS_LIBRARY":
    "Тайтл уже существует в этой библиотеке.",
  "error.reason.LIBRARY_TITLE_DIRECTORY_NOT_DIRECTORY":
    "Каталог тайтла не является каталогом.",
  "error.reason.LIBRARY_TITLE_FOLDER_OVERLAPS_ANOTHER_TITLE":
    "Папка тайтла пересекается с другим тайтлом; удаление медиа запрещено.",
  "error.reason.LIBRARY_TITLE_IDENTITY_REQUIRED":
    "Требуются идентификационные данные тайтла.",
  "error.reason.LIBRARY_TITLE_ID_RENAME_REQUEST_DUPLICATED":
    "В запросе переименования повторяется идентификатор тайтла.",
  "error.reason.LIBRARY_TITLE_LIBRARY_SCAN_ONLY_SUPPORTED_EPISODIC_TITLES":
    "Сканирование библиотеки тайтла поддерживается только для тайтлов с эпизодами.",
  "error.reason.LIBRARY_TITLE_NO_OWNED_FOLDER":
    "У тайтла нет принадлежащей ему папки.",
  "error.reason.LIBRARY_TITLE_NO_RECORDED_MEDIA":
    "У тайтла нет зарегистрированных медиафайлов.",
  "error.reason.LIBRARY_TITLE_REQUIRED":
    "Для переименования требуется хотя бы один тайтл.",
  "error.reason.LIBRARY_UNCLOSED_EPISODE_FOLDER_TOKEN":
    "Незакрытый токен папки эпизода.",
  "error.reason.LIST_ACCOUNT_DISABLED": "Учётная запись отключена.",
  "error.reason.LIST_ACCOUNT_LINK_NOT_FOUND":
    "Связь с учётной записью списков не найдена.",
  "error.reason.LIST_ACCOUNT_NOT_FOUND": "Учётная запись списков не найдена.",
  "error.reason.LIST_ADD_SEARCH_NEED_TITLE_MANAGEMENT_PERMISSION":
    "Для добавления и поиска требуется разрешение на управление тайтлами.",
  "error.reason.LIST_COULD_NOT_START_ACCOUNT_LINK":
    "Не удалось начать привязку учётной записи.",
  "error.reason.LIST_DEPARTURE_CHANGES_NEED_TITLE_MANAGEMENT_PERMISSION":
    "Для изменения действий при выходе из списка требуется разрешение на управление тайтлами.",
  "error.reason.LIST_EXCLUSION_NOT_FOUND": "Исключение из списка не найдено.",
  "error.reason.LIST_OWNER_DISABLED": "Владелец списка отключён.",
  "error.reason.LIST_OWNER_NOT_FOUND": "Владелец списка не найден.",
  "error.reason.LIST_PERSONAL_LISTS_NOT_ALLOWED_MEMBER":
    "Личные списки не разрешены для этого участника.",
  "error.reason.LIST_PROVIDER_NOT_FOUND": "Провайдер списков не найден.",
  "error.reason.LIST_SUBSCRIPTION_CHANGED_NOT_FOUND":
    "Изменённая подписка на список не найдена.",
  "error.reason.LIST_SUBSCRIPTION_NOT_FOUND": "Подписка на список не найдена.",
  "error.reason.LIST_TITLE_NOT_FOUND": "Тайтл не найден.",
  "error.reason.LIST_USER_NOT_FOUND": "Пользователь не найден.",
  "error.reason.LOCATION_CATALOG_CANNOT_MAP_FILE_OUTSIDE_RECORDED_FOLDER":
    "Каталог не может сопоставить файл за пределами сохранённой папки выбранного элемента. Сначала исправьте сопоставление его папки.",
  "error.reason.LOCATION_CATALOG_PATH_AMBIGUOUS":
    "Путь в каталоге для выбранного элемента неоднозначен. Перед перемещением исправьте сопоставление его папки.",
  "error.reason.LOCATION_CONFIRMED_PLAN_NO_INSTRUCTIONS_TITLE":
    "В подтверждённом плане нет инструкций для тайтла с выбранным значением.",
  "error.reason.LOCATION_COULD_NOT_COMPARE":
    "Не удалось сравнить выбранный элемент. Убедитесь, что он доступен для чтения, и повторите попытку; источник сохранён.",
  "error.reason.LOCATION_DESTINATION_CHANGED_DURING_COMPARISON":
    "Файл назначения изменился во время сравнения. Источник сохранён; повторите попытку, когда файлы перестанут изменяться.",
  "error.reason.LOCATION_DESTINATION_COPY_NO_LONGER_HOLDS_CONTENT_MOVE_PROVED_IDENTICAL_SOURCE":
    "Копия в месте назначения больше не содержит данные, тождественность которых источнику была подтверждена при этом перемещении. Источник сохранён; повторите попытку, чтобы сравнить файлы заново.",
  "error.reason.LOCATION_DESTINATION_OCCUPIED":
    "Промежуточный путь назначения с выбранным значением занят файлом, который не был создан этим перемещением. Оба файла сохранены; переместите или удалите мешающий файл и повторите попытку.",
  "error.reason.LOCATION_DESTINATION_REQUIRED":
    "Выберите библиотеку или корневой каталог назначения.",
  "error.reason.LOCATION_DESTINATION_TITLE_NOT_FOUND":
    "Целевой тайтл не найден.",
  "error.reason.LOCATION_EXISTING_PATH_NOT_REGULAR_FILE":
    "Существующий путь не является обычным файлом. Устраните конфликт пути и повторите попытку; источник сохранён.",
  "error.reason.LOCATION_FILE_CHANGED_BEFORE_SOURCE_CLEANUP":
    "Файл изменился до очистки источника. Источник сохранён; повторите попытку, чтобы сравнить его заново.",
  "error.reason.LOCATION_FILE_CHANGED_WHILE_BEING_COMPARED":
    "Файл изменился во время сравнения. Источник сохранён; повторите попытку, когда файлы перестанут изменяться.",
  "error.reason.LOCATION_FILE_CHANGED_WHILE_HASHING":
    "Файл изменился во время вычисления хеша.",
  "error.reason.LOCATION_FOLDER_ALREADY_OWNED_TITLE":
    "Папка с выбранным значением уже принадлежит тайтлу с выбранным значением; выберите обмен папками или передачу владения.",
  "error.reason.LOCATION_FOLDER_NOT_INSIDE_ROOT_LIBRARY":
    "Папка с выбранным значением не находится внутри корневого каталога библиотеки с выбранным значением.",
  "error.reason.LOCATION_FOLDER_NOT_OWNED_ANOTHER_TITLE":
    "Папка с выбранным значением не принадлежит другому тайтлу; выбранный способ разрешения конфликта неприменим.",
  "error.reason.LOCATION_FOLDER_NOT_TITLE_BOUNDARY":
    "Папка с выбранным значением является корневым каталогом библиотеки, а не папкой тайтла.",
  "error.reason.LOCATION_FOLDER_PATH_REQUIRED": "Требуется путь к папке.",
  "error.reason.LOCATION_FULL_HASH_BACKFILL_ALREADY_RUNNING":
    "Фоновое заполнение полных хешей уже выполняется.",
  "error.reason.LOCATION_FULL_HASH_BACKFILL_CANCELLED":
    "Фоновое заполнение полных хешей отменено.",
  "error.reason.LOCATION_LIBRARY_NOT_FOUND": "Библиотека не найдена.",
  "error.reason.LOCATION_LIBRARY_ROOT_NOT_FOUND":
    "Корневой каталог библиотеки не найден.",
  "error.reason.LOCATION_MERGE_TITLE_INTO_BLOCKED":
    "Объединение тайтла с выбранным значением с выбранным целевым тайтлом заблокировано.",
  "error.reason.LOCATION_MERGE_UNAVAILABLE":
    "Для объединения нет допустимого сопоставления идентификационных данных.",
  "error.reason.LOCATION_MORE_ONE_CATALOG_RECORD_WOULD_POINT":
    "На выбранный путь будут ссылаться несколько записей каталога. Устраните этот конфликт каталога перед подтверждением ручного перемещения.",
  "error.reason.LOCATION_OPERATION_NOT_FOUND":
    "Операция с расположением не найдена.",
  "error.reason.LOCATION_OPERATION_SOURCE_LIBRARIES_UNAVAILABLE":
    "Исходные библиотеки операции с расположением недоступны.",
  "error.reason.LOCATION_OPERATION_STILL_RUNNING":
    "Операция с расположением с выбранным значением ещё выполняется; отмените её вместо того, чтобы отказываться от неё.",
  "error.reason.LOCATION_OWNERSHIP_BUSY":
    "Эти элементы сейчас заняты операцией с расположением.",
  "error.reason.LOCATION_PATH_COMPONENT_INVALID":
    "Папка с выбранным значением не должна содержать компоненты . или ...",
  "error.reason.LOCATION_PATH_NOT_FOUND": "Путь не найден.",
  "error.reason.LOCATION_PATH_OUTSIDE_LIBRARY_ROOT":
    "После разрешения выбранного пути папка оказывается за пределами корневого каталога своей библиотеки.",
  "error.reason.LOCATION_RECORDED_FOLDER_LIBRARY_ROOT_NOT_FOLDER_SWAP":
    "Сохранённая папка выбранного элемента является корневым каталогом библиотеки, а не папкой для обмена; вместо этого передайте владение папкой.",
  "error.reason.LOCATION_RECOVERY_ALREADY_RUNNING":
    "Восстановление уже выполняется.",
  "error.reason.LOCATION_RECOVERY_COMMIT_RECEIPT_REQUIRED":
    "Восстановление уже зафиксировано; отсутствие квитанции не даёт права создавать ещё одну копию.",
  "error.reason.LOCATION_RECOVERY_CONTENT_DISCOVERY_LIMIT_REACHED":
    "Достигнут лимит обнаружения содержимого.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_ANCESTOR_NOT_FOUND":
    "Для пути назначения восстановления не найден ни один существующий родительский каталог.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_BOUNDARY_INVALID":
    "Путь назначения восстановления не является допустимой папкой тайтла.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_CHANGED":
    "Путь назначения изменился во время обнаружения содержимого.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_CLAIM_FAILED":
    "Не удалось закрепить путь назначения; исходные и промежуточные файлы сохранены.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_CONTENT_MISMATCH":
    "Содержимое файла назначения не совпадает; оба файла сохранены.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_HASH_AMBIGUOUS":
    "Несколько файлов назначения соответствуют хешу отсутствующего источника.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_HASH_NOT_FOUND":
    "Не найден файл назначения с хешем полного содержимого, совпадающим с отсутствующим источником.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_INVENTORY_LIMIT_REACHED":
    "Превышен лимит создаваемой описи места назначения.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_NOT_FOUND":
    "Путь назначения восстановления не найден.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_OUTSIDE_LIBRARY":
    "Путь назначения находится за пределами библиотеки тайтла.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_OWNERSHIP_CONFLICT":
    "Пересекающаяся папка назначения принадлежит другому тайтлу.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_PARENT_NOT_DIRECTORY":
    "Родительский путь назначения восстановления не является каталогом.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_PARENT_REQUIRED":
    "У пути назначения нет родительского каталога.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_PATH_INVALID":
    "Пути назначения восстанавливаемых файлов конфликтуют или выходят за пределы папки тайтла.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_RECORD_CONFLICT":
    "Путь назначения уже связан с другой записью медиа.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_ROOT_UNPROVEN":
    "Нет подтверждённого корневого каталога назначения.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_UNPROVEN":
    "Нет подтверждённого пути назначения.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_UNRELATED_CONTENT":
    "Созданное место назначения содержит посторонние данные; принадлежность остаётся неустановленной.",
  "error.reason.LOCATION_RECOVERY_DOT_SEGMENT_PATH_FORBIDDEN":
    "Восстановление не следует по путям с компонентами . или ...",
  "error.reason.LOCATION_RECOVERY_FILE_IDENTITY_UNAVAILABLE":
    "Идентификационные данные объекта файловой системы недоступны.",
  "error.reason.LOCATION_RECOVERY_FILE_IDENTITY_UNSUPPORTED":
    "Идентификационные данные объекта файловой системы не поддерживаются.",
  "error.reason.LOCATION_RECOVERY_FILE_NOT_IN_PLAN":
    "Восстанавливаемого файла нет в сохранённом плане.",
  "error.reason.LOCATION_RECOVERY_FILE_OWNERSHIP_CONFLICT":
    "Другой тайтл отслеживает исходный файл или файл назначения.",
  "error.reason.LOCATION_RECOVERY_FOLDER_DISCOVERY_LIMIT_REACHED":
    "Достигнут лимит обнаружения папок; автоматическое исправление не выполняется.",
  "error.reason.LOCATION_RECOVERY_FOLDER_IDENTITY_AMBIGUOUS":
    "Несколько папок идентифицируют этот тайтл.",
  "error.reason.LOCATION_RECOVERY_FOLDER_IDENTITY_CHANGED":
    "Точные идентификационные данные папки изменились; принадлежность остаётся неустановленной.",
  "error.reason.LOCATION_RECOVERY_FOLDER_OWNERSHIP_UNPROVEN":
    "Принадлежность не подтверждена ни медиафайлами, ни папкой с точным идентификатором.",
  "error.reason.LOCATION_RECOVERY_FOLDER_TITLE_IDENTITY_MISMATCH":
    "Идентификационные данные отслеживаемой папки противоречат тайтлу.",
  "error.reason.LOCATION_RECOVERY_FULL_HASH_REQUIRED":
    "Отсутствует полный хеш.",
  "error.reason.LOCATION_RECOVERY_GENERATED_DESTINATION_IDENTITY_MISMATCH":
    "Созданное место назначения не является папкой, записанной для этого восстановления.",
  "error.reason.LOCATION_RECOVERY_GENERATED_DESTINATION_NOT_DIRECTORY":
    "Созданное место назначения не является каталогом.",
  "error.reason.LOCATION_RECOVERY_GENERATED_DESTINATION_UNCLAIMED":
    "Созданное место назначения уже существует, но подтверждения его принадлежности нет.",
  "error.reason.LOCATION_RECOVERY_IDENTITY_EVIDENCE_CONFLICT":
    "Свидетельства идентификации папки противоречат друг другу.",
  "error.reason.LOCATION_RECOVERY_IDENTITY_SIDECAR_INVALID":
    "Сопутствующий файл идентификации не поддерживается или слишком велик.",
  "error.reason.LOCATION_RECOVERY_INDEPENDENT_COPY_REQUIRED":
    "Для восстановления требуется независимая копия в месте назначения, а не жёсткая ссылка.",
  "error.reason.LOCATION_RECOVERY_MEDIA_NOT_LOOSE_AT_ANCESTOR":
    "Медиафайлы не лежат непосредственно в повреждённом родительском каталоге.",
  "error.reason.LOCATION_RECOVERY_MEDIA_PARENT_REQUIRED":
    "У медиафайла нет родительской папки.",
  "error.reason.LOCATION_RECOVERY_NATIVE_ABSOLUTE_PATH_REQUIRED":
    "Для восстановления требуется абсолютный путь в собственном формате файловой системы.",
  "error.reason.LOCATION_RECOVERY_OWNERSHIP_CHANGED":
    "Принадлежность тайтлу изменилась; оригиналы сохранены.",
  "error.reason.LOCATION_RECOVERY_PATH_UNAVAILABLE":
    "Путь восстановления недоступен.",
  "error.reason.LOCATION_RECOVERY_PLAN_NOT_FOUND":
    "План восстановления не найден.",
  "error.reason.LOCATION_RECOVERY_PLAN_UNSUPPORTED":
    "Неподдерживаемый план восстановления.",
  "error.reason.LOCATION_RECOVERY_RECYCLE_STORAGE_OVERLAP":
    "Путь назначения восстановления пересекается с настроенным хранилищем корзины.",
  "error.reason.LOCATION_RECOVERY_REGULAR_FILE_REQUIRED":
    "Для восстановления требуется обычный файл.",
  "error.reason.LOCATION_RECOVERY_REPARSE_POINT_FORBIDDEN":
    "Восстановление не следует через точки повторной обработки.",
  "error.reason.LOCATION_RECOVERY_ROOTS_CHANGED":
    "Корневые каталоги библиотеки изменились; для восстановления нужен новый план.",
  "error.reason.LOCATION_RECOVERY_ROOTS_REQUIRED":
    "В библиотеке тайтла не настроены корневые каталоги.",
  "error.reason.LOCATION_RECOVERY_ROOT_CORRUPTION_REQUIRED":
    "Проблема принадлежности тайтла не связана с повреждением корневого каталога.",
  "error.reason.LOCATION_RECOVERY_ROOT_NOT_FOUND":
    "Корневой каталог восстановления не найден.",
  "error.reason.LOCATION_RECOVERY_SELECTED_ROOT_NOT_FOUND":
    "Выбранный корневой каталог не найден.",
  "error.reason.LOCATION_RECOVERY_SELECTED_ROOT_OUTSIDE_ANCESTOR":
    "Выбранный корневой каталог не находится ниже повреждённого родительского каталога.",
  "error.reason.LOCATION_RECOVERY_SOURCE_AND_DESTINATION_NOT_FOUND":
    "Не найдены ни источник, ни файл назначения.",
  "error.reason.LOCATION_RECOVERY_SOURCE_CHANGED":
    "Источник изменился во время проверки.",
  "error.reason.LOCATION_RECOVERY_SOURCE_CONTENT_PROOF_REQUIRED":
    "Для отсутствующего источника нет подтверждения полного содержимого.",
  "error.reason.LOCATION_RECOVERY_SOURCE_FILENAME_REQUIRED":
    "Имя исходного файла недоступно.",
  "error.reason.LOCATION_RECOVERY_SOURCE_FULL_HASH_REQUIRED":
    "Для отсутствующего источника нет подтверждённого полного хеша.",
  "error.reason.LOCATION_RECOVERY_SOURCE_LOCATION_UNPROVEN":
    "Отслеживаемый источник не находится ни в подтверждённой папке, ни непосредственно в повреждённом родительском каталоге.",
  "error.reason.LOCATION_RECOVERY_SOURCE_PARENT_UNAVAILABLE":
    "Родительский каталог источника недоступен.",
  "error.reason.LOCATION_RECOVERY_SYMLINK_FORBIDDEN":
    "Восстановление не следует по символическим ссылкам.",
  "error.reason.LOCATION_RECOVERY_TITLE_CHANGED":
    "Тайтл изменился во время восстановления; исходные сопоставления сохранены.",
  "error.reason.LOCATION_RECOVERY_TITLE_NOT_FOUND": "Тайтл не найден.",
  "error.reason.LOCATION_RECOVERY_TRACKED_MEDIA_CHANGED":
    "Отслеживаемые медиафайлы изменились; оригиналы сохранены.",
  "error.reason.LOCATION_RELATED_MEDIA_FILE_COULD_NOT_TRANSFERRED":
    "Не удалось перенести связанный медиафайл. Его сопутствующие файлы сохранены.",
  "error.reason.LOCATION_RELATED_MEDIA_TRANSFER_INTERRUPTED":
    "Перенос связанного медиафайла прерван. Источник сохранён.",
  "error.reason.LOCATION_ROOT_MOVE_SPANS_ONE_LIBRARY":
    "Перенос корня выполняется в пределах одной библиотеки; для выбранных тайтлов из нескольких библиотек укажите целевую библиотеку.",
  "error.reason.LOCATION_ROOT_NEW_PATH_REQUIRED":
    "Выберите новый путь для этого корня.",
  "error.reason.LOCATION_ROOT_NOT_FOUND":
    "Корневая папка в библиотеке не найдена.",
  "error.reason.LOCATION_SELECTED_FOLDER_OVERLAPS_ANOTHER_TITLE_S_FOLDER":
    "Выбранная папка пересекается с папкой другого тайтла; принадлежность файлов не изменена.",
  "error.reason.LOCATION_SELECTION_NOTHING_MOVE":
    "В выбранных элементах нечего переносить.",
  "error.reason.LOCATION_SOURCE_CHANGED_BEFORE_CLEANUP":
    "Источник изменился перед очисткой и был сохранён.",
  "error.reason.LOCATION_SOURCE_CHANGED_DURING_FILE_PLACEMENT":
    "Источник изменился во время размещения файла.",
  "error.reason.LOCATION_SOURCE_CHANGED_DURING_TRANSFER":
    "Источник изменился во время переноса и был сохранён. Повторите попытку, когда файл перестанет изменяться.",
  "error.reason.LOCATION_SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP":
    "Источник или место назначения изменились перед очисткой.",
  "error.reason.LOCATION_SOURCE_DESTINATION_SAME_PATH":
    "Пути источника и места назначения совпадают.",
  "error.reason.LOCATION_STALE_PLAN":
    "План изменился. Повторите предварительный просмотр.",
  "error.reason.LOCATION_TITLE_OWNS_NO_FOLDER_SWAP":
    "У выбранного тайтла нет папки для обмена; вместо этого назначьте ему эту папку.",
  "error.reason.LOCATION_TITLE_SELECTION_REQUIRED":
    "Выберите хотя бы один тайтл для переноса.",
  "error.reason.LOCATION_TRANSFERRED_FILE_NEEDS_VERIFIED_AGAIN_BEFORE_CATALOG_ENTRY_UPDATED":
    "Перед обновлением записи каталога необходимо повторно проверить перенесённый файл.",
  "error.reason.LOCATION_TRANSFERRED_FILE_NOT_FINISHED_VERIFICATION":
    "Проверка перенесённого файла ещё не завершена. Его запись в каталоге не изменена.",
  "error.reason.LOCATION_VERIFICATION_FAILED":
    "Проверка выбранного элемента не удалась.",
  "error.reason.LOCATION_VERIFICATION_MISMATCH":
    "Файл в месте назначения не прошёл проверку.",
  "error.reason.MAINTENANCE_ACTION_CONFIGURATION_INVALID":
    "Недопустимая конфигурация действия обслуживания.",
  "error.reason.MAINTENANCE_ACTION_INVALID":
    "Это действие обслуживания недопустимо для правила уровня тайтла.",
  "error.reason.MAINTENANCE_ACTION_NOT_STORABLE":
    "Действие обслуживания нельзя сохранить.",
  "error.reason.MAINTENANCE_ACTION_UNSUPPORTED":
    "Это действие обслуживания не поддерживается для тайтлов.",
  "error.reason.MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID":
    "Выбранный элемент уже выполнял действия в библиотеке, и его удаление сотрёт историю этих действий. Вместо этого отключите его: правило немедленно остановится, а история сохранится.",
  "error.reason.MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE":
    "Неоднозначно определены файлы, охватываемые обслуживанием.",
  "error.reason.MAINTENANCE_ARM_COUNT_MISMATCH":
    "Количество кандидатов изменилось. Проверьте текущее количество перед разрешением удаления.",
  "error.reason.MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON":
    "Для удаления дочернего элемента требуется серия или сезон.",
  "error.reason.MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE":
    "Выбранный элемент сейчас отслеживает тайтлы, и его удаление бесследно удалит сведения об отслеживании. Вместо этого отключите его или сначала очистите список кандидатов.",
  "error.reason.MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE":
    "У действия DeleteFiles несовместимое предварительное условие Unmonitor.",
  "error.reason.MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP":
    "Перед действием DeleteFiles требуется шаг Unmonitor.",
  "error.reason.MAINTENANCE_EXCLUSION_NOT_FOUND":
    "Исключение обслуживания не найдено.",
  "error.reason.MAINTENANCE_FIELD_REQUIRED":
    "Обязательное значение не должно быть пустым.",
  "error.reason.MAINTENANCE_FILE_OWNERSHIP_CHANGED":
    "Принадлежность файла обслуживания изменилась.",
  "error.reason.MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED":
    "Набор файлов для обслуживания изменился; требуется повторная оценка.",
  "error.reason.MAINTENANCE_GRACE_PERIOD_INVALID":
    "Период ожидания должен составлять ноль или более дней.",
  "error.reason.MAINTENANCE_GRACE_PERIOD_OUT_OF_RANGE":
    "Период ожидания не должен превышать установленный предел в днях (десять лет).",
  "error.reason.MAINTENANCE_MATCHER_READS_STORAGE_FACTS_MUST_SELECT_CONFIGURED_LIBRARY_ROOT":
    "Условие, использующее сведения о хранилище, должно выбирать настроенный корень библиотеки.",
  "error.reason.MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY":
    "У фильмов нет списка серий.",
  "error.reason.MAINTENANCE_PERSON_TARGETING_PERMISSION_REQUIRED":
    "Это правило использует сведения о конкретных людях, поэтому для его сохранения требуется право управления разрешениями. Попросите администратора сохранить правило или перепишите его без этих сведений.",
  "error.reason.MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED":
    "Для предварительного просмотра запрошено слишком много тайтлов.",
  "error.reason.MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS":
    "Для предварительного просмотра требуется от одного до пятидесяти объектов.",
  "error.reason.MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE":
    "Объект предварительного просмотра отсутствует, находится вне библиотек правила или не относится к выбранным тайтлам и области действия.",
  "error.reason.MAINTENANCE_QUALITY_PROFILE_ACTION_NO_TARGET_PROFILE":
    "Для действия изменения профиля качества не указан целевой профиль.",
  "error.reason.MAINTENANCE_RULE_CHANGED_WHILE_CONFIRMING":
    "Правило изменилось во время подтверждения; проверьте его ещё раз перед разрешением удаления.",
  "error.reason.MAINTENANCE_RULE_CONFIGURATION_INVALID":
    "Перед разрешением удаления необходимо повторно проверить конфигурацию правила; откройте подтверждение заново.",
  "error.reason.MAINTENANCE_RULE_EVALUATION_FAILED":
    "Не удалось выполнить оценку правила.",
  "error.reason.MAINTENANCE_RULE_FAILED_COMPILE":
    "Не удалось скомпилировать правило обслуживания.",
  "error.reason.MAINTENANCE_RULE_SET_NOT_FOUND":
    "Набор правил обслуживания не найден.",
  "error.reason.MAINTENANCE_RULE_SHIPS_SCRYER":
    "Это правило входит в состав Scryer: его условие и действия изменить нельзя. Можно изменить период ожидания, библиотеки и состояние.",
  "error.reason.MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED":
    "Это правило входит в состав Scryer и не может быть удалено. Вместо этого отключите его.",
  "error.reason.MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED":
    "Это правило входит в состав Scryer и не может быть переименовано.",
  "error.reason.MAINTENANCE_RULE_SOURCE_INVALID":
    "Недопустимый исходный код правила обслуживания.",
  "error.reason.MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN":
    "Действие этого правила ничего не изменяет, поэтому оно доступно только в режиме проверки без применения.",
  "error.reason.MAINTENANCE_RULE_VALIDATION_FAILED":
    "Проверка правила не удалась.",
  "error.reason.MAINTENANCE_SEQUENCE_ACTIONS_INVALID":
    "Действия последовательности должны использовать исполнитель последовательностей.",
  "error.reason.MAINTENANCE_SEQUENCE_STEP_PARAMETERS_INVALID":
    "Недопустимые параметры шага последовательности обслуживания.",
  "error.reason.MAINTENANCE_STORAGE_ROOT_NOT_CONFIGURED":
    "Выбранный корень хранилища не настроен.",
  "error.reason.MAINTENANCE_SUBJECT_ID_REQUIRED":
    "Требуется идентификатор объекта обслуживания.",
  "error.reason.MAINTENANCE_SUBJECT_KIND_INVALID":
    "Недопустимый тип объекта обслуживания.",
  "error.reason.MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND":
    "Объект обслуживания не найден или больше не относится к этому тайтлу.",
  "error.reason.MAINTENANCE_TAG_ACTION_NO_TAGS_CONFIGURED":
    "Для действия назначения тегов не заданы теги.",
  "error.reason.MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST":
    "Целевой профиль качества не существует.",
  "error.reason.MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE":
    "Идентификатор кандидата на отключение мониторинга не соответствует его тайтлу.",
  "error.reason.MEDIA_ANALYSIS_INVALID":
    "Недопустимый результат анализа медиафайла.",
  "error.reason.MEDIA_ARTWORK_WORKER_COUNT_INVALID":
    "Количество обработчиков изображений должно равняться двум или четырём.",
  "error.reason.MEDIA_CACHED_AVIF_INVALID":
    "Кодировщик кешированных JPEG вернул недопустимый AVIF.",
  "error.reason.MEDIA_CACHED_IMAGE_NOT_JPEG":
    "Кешированное изображение не является JPEG.",
  "error.reason.MEDIA_CACHED_JPEG_DECODE_FAILED":
    "Не удалось декодировать кешированный JPEG.",
  "error.reason.MEDIA_CACHED_JPEG_DIMENSIONS_INVALID":
    "Недопустимый размер кешированного JPEG или целевая ширина.",
  "error.reason.MEDIA_CACHED_JPEG_KIND_UNSUPPORTED":
    "Неподдерживаемый тип кешированного JPEG.",
  "error.reason.MEDIA_CACHED_JPEG_TOO_LARGE":
    "Кешированный JPEG превышает ограничение размера источника.",
  "error.reason.MEDIA_CLAIM_EXTENSION_NOT_FUTURE":
    "Продление срока должно указывать время в будущем.",
  "error.reason.MEDIA_CLAIM_EXTENSION_NOT_LONGER":
    "Продление должно увеличивать действующий ограниченный срок хранения.",
  "error.reason.MEDIA_COLLECTION_NOT_FOUND": "Коллекция не найдена.",
  "error.reason.MEDIA_COLLECTION_UPDATE_EMPTY":
    "Необходимо указать хотя бы одно поле коллекции.",
  "error.reason.MEDIA_DESTINATION_LIBRARY_NOT_FOUND":
    "Целевая библиотека не найдена.",
  "error.reason.MEDIA_DISC_ANALYSIS_REQUIRES_REVIEW_BEFORE_EPISODES_MADE_AVAILABLE":
    "Прежде чем серии станут доступны, необходимо проверить результаты анализа диска.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPINGS_REQUIRE_ISO_IMAGE":
    "Для сопоставления серий диска требуется ISO-образ.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPING_DUPLICATE":
    "Повторяющееся сопоставление серии диска.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPING_INVALID":
    "Недопустимое или неполное сопоставление серии диска.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPING_LIMIT_EXCEEDED":
    "Превышено ограничение количества сопоставлений серий диска.",
  "error.reason.MEDIA_DISC_EPISODE_TITLE_MISMATCH":
    "Серия диска относится к другому тайтлу или больше не существует.",
  "error.reason.MEDIA_DISC_SELECTION_REQUIRES_ISO_IMAGE":
    "Для выбора содержимого диска требуется ISO-образ.",
  "error.reason.MEDIA_DISC_TITLE_RUNTIME_SECONDS_IMPLAUSIBLE_EPISODE":
    "Длительность выбранной программы диска неправдоподобна для указанной серии.",
  "error.reason.MEDIA_EACH_EPISODE_DISTINCT_PLAYBACK_TITLE_OCCUR_ONLY_ONE_MAPPING":
    "Каждая серия и каждая отдельная воспроизводимая программа могут присутствовать только в одном сопоставлении.",
  "error.reason.MEDIA_EPISODE_MAPPINGS_REQUIRE_SERIES_TITLE":
    "Для сопоставления серий требуется тайтл сериала.",
  "error.reason.MEDIA_EPISODE_NOT_FOUND": "Серия не найдена.",
  "error.reason.MEDIA_EPISODE_UPDATE_EMPTY":
    "Необходимо указать хотя бы одно поле серии.",
  "error.reason.MEDIA_EXTERNAL_ID_AMBIGUOUS":
    "Внешние идентификаторы уже соответствуют нескольким тайтлам.",
  "error.reason.MEDIA_FACET_LIBRARY_NOT_FOUND":
    "Библиотека этой категории медиа не найдена.",
  "error.reason.MEDIA_FILE_NOT_FOUND": "Медиафайл не найден.",
  "error.reason.MEDIA_FILE_OWNERSHIP_CHANGED":
    "Принадлежность медиафайла изменилась.",
  "error.reason.MEDIA_IMAGE_DECODE_FAILED":
    "Не удалось декодировать изображение.",
  "error.reason.MEDIA_IMAGE_DIMENSIONS_EMPTY":
    "Размеры изображения должны быть ненулевыми.",
  "error.reason.MEDIA_IMAGE_FORMAT_DETECTION_FAILED":
    "Не удалось определить формат изображения.",
  "error.reason.MEDIA_IMAGE_FORMAT_UNSUPPORTED":
    "Неподдерживаемый формат изображения.",
  "error.reason.MEDIA_IMAGE_NO_RECOGNIZED_DISC_TITLES":
    "В образе нет распознанных программ диска.",
  "error.reason.MEDIA_IMAGE_PROXY_CONTENT_TYPE_MISMATCH":
    "Данные ответа прокси изображений не соответствуют заявленному типу изображения.",
  "error.reason.MEDIA_IMAGE_PROXY_CONTENT_TYPE_UNSUPPORTED":
    "Неподдерживаемый тип содержимого прокси изображений.",
  "error.reason.MEDIA_IMAGE_PROXY_RESPONSE_TOO_LARGE":
    "Ответ прокси изображений слишком большой.",
  "error.reason.MEDIA_IMAGE_PROXY_SOURCE_UNAPPROVED":
    "Неодобренный источник прокси изображений.",
  "error.reason.MEDIA_IMAGE_SOURCE_REJECTED":
    "Источник изображения не прошёл проверку общедоступного HTTP URL.",
  "error.reason.MEDIA_LIBRARY_NOT_FOUND": "Библиотека не найдена.",
  "error.reason.MEDIA_LIBRARY_PATH_NOT_DIRECTORY":
    "Путь к библиотеке должен указывать на каталог.",
  "error.reason.MEDIA_LIBRARY_PATH_UNAVAILABLE": "Ошибка пути к библиотеке.",
  "error.reason.MEDIA_LIBRARY_ROOT_ALREADY_CONFIGURED":
    "Этот корневой каталог библиотеки уже настроен.",
  "error.reason.MEDIA_LIBRARY_ROOT_NOT_FOUND":
    "Корневой каталог библиотеки не найден.",
  "error.reason.MEDIA_LIBRARY_ROOT_PATH_EMPTY":
    "Путь к корневому каталогу библиотеки не может быть пустым.",
  "error.reason.MEDIA_LIBRARY_ROOT_REFERENCED":
    "Нельзя удалить корневой каталог библиотеки, пока на него ссылаются тайтлы.",
  "error.reason.MEDIA_LIFECYCLE_CLAIM_NOT_LIVE":
    "Заявка жизненного цикла больше не активна.",
  "error.reason.MEDIA_LIST_ACCOUNT_NOT_FOUND":
    "Учётная запись списка не найдена.",
  "error.reason.MEDIA_LIST_EXCLUSION_NOT_FOUND":
    "Исключение списка не найдено.",
  "error.reason.MEDIA_LIST_MOVIE_SELECTION_MISSING":
    "Не выбран фильм из списка.",
  "error.reason.MEDIA_LIST_SUBSCRIPTION_NOT_FOUND":
    "Подписка на список не найдена.",
  "error.reason.MEDIA_LOCATION_OPERATION_NOT_FOUND":
    "Операция с расположением не найдена.",
  "error.reason.MEDIA_MAPPED_DISC_TITLE_INCOMPLETE_UNSUPPORTED_ANALYSIS":
    "Анализ сопоставленного тайтла диска с выбранным значением неполон или не поддерживается.",
  "error.reason.MEDIA_MAPPED_DISC_TITLE_NO_ESTABLISHED_RUNTIME":
    "Для сопоставленного тайтла диска с выбранным значением не определена продолжительность.",
  "error.reason.MEDIA_MAPPED_DISC_TITLE_NO_LONGER_EXISTS":
    "Сопоставленный тайтл диска с выбранным значением больше не существует; проверьте сопоставление.",
  "error.reason.MEDIA_MAPPED_EPISODE_DOES_NOT_BELONG_TITLE":
    "Сопоставленный эпизод не относится к этому тайтлу.",
  "error.reason.MEDIA_MAP_EACH_COMPLETE_DISC_TITLE_ONE_EPISODE":
    "Сопоставьте каждый полный тайтл диска с одним эпизодом; для объединённых эпизодов нужны отдельные тайтлы воспроизведения, созданные при авторинге.",
  "error.reason.MEDIA_MOST_SEARCH_LANGUAGES_ALLOWED":
    "Допускается не более выбранного количества языков поиска.",
  "error.reason.MEDIA_MOVIE_IDENTITY_UNSUPPORTED":
    "У выбранного фильма нет поддерживаемого идентификатора.",
  "error.reason.MEDIA_NATIVE_DIAGNOSTICS_NOT_COMPILED_INTO_TARGET":
    "Нативная диагностика не включена в эту сборку.",
  "error.reason.MEDIA_NUMERIC_DISC_TITLE_IDENTITY_INVALID":
    "Недопустимый числовой идентификатор тайтла диска.",
  "error.reason.MEDIA_PENDING_IMPORT_UNRESOLVED":
    "Не удалось разрешить ожидающий импорт для тайтла.",
  "error.reason.MEDIA_PLAYBACK_CONNECTION_MISMATCH":
    "ID подключения в сопоставлении воспроизведения не совпадает с целевым подключением для сохранения.",
  "error.reason.MEDIA_RECOVERY_CANCELED_BEFORE_COMMIT":
    "Восстановление отменено до фиксации каталога; исходные сопоставления сохранены.",
  "error.reason.MEDIA_RECOVERY_CATALOG_EVIDENCE_LIMIT_EXCEEDED":
    "Превышен лимит подтверждающих данных каталога восстановления; изменения не внесены.",
  "error.reason.MEDIA_RECOVERY_IDENTITY_CHANGED":
    "Идентификатор объекта восстановления изменился.",
  "error.reason.MEDIA_RECOVERY_MEDIA_CHANGED":
    "Медиа изменились во время восстановления.",
  "error.reason.MEDIA_RECOVERY_NOT_REPLANNABLE":
    "В текущем состоянии нельзя повторно составить план восстановления.",
  "error.reason.MEDIA_RECOVERY_OPERATION_INVALID":
    "Недопустимая операция восстановления.",
  "error.reason.MEDIA_RECOVERY_OPERATION_NOT_FOUND":
    "Операция восстановления не найдена.",
  "error.reason.MEDIA_RECOVERY_PLAN_UNRESOLVED":
    "План восстановления изменился или по-прежнему не согласован.",
  "error.reason.MEDIA_RECOVERY_ROOT_MISSING":
    "Не указан корневой каталог восстановления.",
  "error.reason.MEDIA_RECOVERY_STAGING_EVIDENCE_LIMIT_EXCEEDED":
    "Превышен лимит подтверждающих данных промежуточного этапа восстановления.",
  "error.reason.MEDIA_RECOVERY_TITLE_NOT_FOUND":
    "Тайтл для восстановления не найден.",
  "error.reason.MEDIA_RECOVERY_VERIFICATION_MISMATCH":
    "Для восстановления необходимы совпадающие подтверждения полной проверки.",
  "error.reason.MEDIA_RECOVERY_VERIFICATION_RECEIPT_REQUIRED":
    "У файла восстановления нет надёжно сохранённого подтверждения проверки.",
  "error.reason.MEDIA_REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION":
    "Зарегистрированный путь к медиа изменился до проверки.",
  "error.reason.MEDIA_RENAME_TARGET_PATH_REQUIRED":
    "Для действий перемещения требуется целевой путь переименования.",
  "error.reason.MEDIA_REQUESTED_DISC_TITLE_COULD_NOT_ESTABLISHED":
    "Не удалось определить запрошенный тайтл диска; проверьте статус его анализа.",
  "error.reason.MEDIA_REQUESTED_DISC_TITLE_DOES_NOT_EXIST":
    "Запрошенный тайтл диска не существует.",
  "error.reason.MEDIA_REQUEST_ADVANCED_MONITORING_ONLY_VALID_SERIES_ANIME_TITLES":
    "Расширенное отслеживание доступно только для сериалов и аниме.",
  "error.reason.MEDIA_REQUEST_APPROVAL_TITLE_CHANGED":
    "Тайтл запроса изменился во время одобрения; повторите попытку.",
  "error.reason.MEDIA_REQUEST_APPROVED_QUALITY_PROFILE_NAME_REQUIRED":
    "Необходимо указать название одобренного профиля качества.",
  "error.reason.MEDIA_REQUEST_APPROVED_QUALITY_PROFILE_REQUIRED":
    "Необходимо указать одобренный профиль качества.",
  "error.reason.MEDIA_REQUEST_CLAIM_EXTENSION_MUST_LENGTHEN_FINITE_LEASE_INTO_FUTURE":
    "Продление заявки должно увеличивать конечный срок действия, устанавливая его в будущем.",
  "error.reason.MEDIA_REQUEST_DO_NOT_OWN_MEDIA_REQUEST":
    "Этот запрос медиа принадлежит не вам.",
  "error.reason.MEDIA_REQUEST_EXTERNAL_IDS_MAP_MULTIPLE_TITLES":
    "Внешние идентификаторы соответствуют нескольким тайтлам.",
  "error.reason.MEDIA_REQUEST_ID_REQUIRED":
    "Необходимо указать ID запроса медиа.",
  "error.reason.MEDIA_REQUEST_LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET":
    "Тип библиотеки не соответствует типу запрошенных медиа.",
  "error.reason.MEDIA_REQUEST_LIBRARY_NOT_FOUND": "Библиотека не найдена.",
  "error.reason.MEDIA_REQUEST_LIFECYCLE_CLAIM_NOT_FOUND":
    "Заявка жизненного цикла не найдена.",
  "error.reason.MEDIA_REQUEST_LIST_MOVIE_APPROVAL_MUST_RETAIN_SELECTED_MOVIE":
    "При одобрении фильма из списка необходимо сохранить выбранный фильм.",
  "error.reason.MEDIA_REQUEST_LIST_MOVIE_REQUEST_MUST_RETAIN_SELECTED_MOVIE":
    "Запрос фильма из списка должен сохранять выбранный фильм.",
  "error.reason.MEDIA_REQUEST_MEDIA_REQUESTS_INVALID":
    "Запросы медиа должны содержать идентификатор SMG, пригодный для поиска.",
  "error.reason.MEDIA_REQUEST_MONITORING_TARGET_REQUIRED":
    "Для расширенного отслеживания требуется хотя бы один сезон или фильм сериала.",
  "error.reason.MEDIA_REQUEST_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND":
    "Тайтл, отсутствующий в ответе с метаданными фильма, не найден.",
  "error.reason.MEDIA_REQUEST_NOT_DISMISSED":
    "Повторно открыть можно только отклонённый запрос медиа.",
  "error.reason.MEDIA_REQUEST_NOT_FOUND": "Запрос не найден.",
  "error.reason.MEDIA_REQUEST_NOT_PENDING":
    "Запрос больше не ожидает рассмотрения.",
  "error.reason.MEDIA_REQUEST_NO_LONGER_PENDING":
    "Запрос медиа больше не ожидает рассмотрения.",
  "error.reason.MEDIA_REQUEST_ONLY_DISMISSED_MEDIA_REQUEST_REOPENED":
    "Повторно открыть можно только отклонённый запрос медиа.",
  "error.reason.MEDIA_REQUEST_QUALITY_PROFILE_ID_REQUIRED":
    "Необходимо указать ID профиля качества.",
  "error.reason.MEDIA_REQUEST_QUALITY_PROFILE_UNKNOWN":
    "Неизвестный профиль качества с выбранным значением.",
  "error.reason.MEDIA_REQUEST_RELEASE_REASON_REQUIRED":
    "Необходимо указать причину освобождения.",
  "error.reason.MEDIA_REQUEST_REQUESTED_QUALITY_PROFILE_NOT_ALLOWED_LIBRARY":
    "Запрошенный профиль качества не разрешён для этой библиотеки.",
  "error.reason.MEDIA_REQUEST_REQUEST_MONITOR_TYPE_UNSUPPORTED":
    "Неподдерживаемый тип отслеживания запроса с выбранным значением.",
  "error.reason.MEDIA_REQUEST_REQUEST_TITLE_REQUIRED":
    "Необходимо указать тайтл запроса.",
  "error.reason.MEDIA_REQUEST_RESOURCE_NOT_FOUND": "Запрос медиа не найден.",
  "error.reason.MEDIA_REQUEST_STATE_NOT_PENDING":
    "Запрос медиа больше не ожидает рассмотрения.",
  "error.reason.MEDIA_REQUEST_TITLE_ALREADY_EXISTS_TARGET_LIBRARY":
    "Тайтл уже существует в целевой библиотеке.",
  "error.reason.MEDIA_REQUEST_TITLE_BEING_MODIFIED":
    "Тайтл изменяется; повторите одобрение.",
  "error.reason.MEDIA_REQUEST_TITLE_CHANGED_DURING_APPROVAL":
    "Тайтл изменился во время одобрения; повторите попытку.",
  "error.reason.MEDIA_REQUEST_TITLE_NOT_FOUND": "Тайтл не найден.",
  "error.reason.MEDIA_SCAN_BATCH_SIZE_INVALID":
    "Размер пакета должен быть больше 0.",
  "error.reason.MEDIA_SEARCH_LANGUAGE_CHOOSE_LANGUAGE_OFFERED_SEARCH_LANGUAGE_PICKER_UNKNOWN":
    "Неизвестный язык поиска с выбранным значением; выберите язык из списка языков поиска.",
  "error.reason.MEDIA_SELECTED_ROOT_CHANGED":
    "Выбранный корневой каталог изменился.",
  "error.reason.MEDIA_SERIES_MOVIE_LINK_NOT_FOUND":
    "Связь с фильмом сериала не найдена.",
  "error.reason.MEDIA_SERVER_BASE_URL_INVALID":
    "Базовый URL медиасервера должен использовать HTTP или HTTPS.",
  "error.reason.MEDIA_SERVER_BASE_URL_QUERY_OR_FRAGMENT":
    "Базовый URL медиасервера не должен содержать параметры запроса или фрагмент.",
  "error.reason.MEDIA_SERVER_BASE_URL_REQUIRED":
    "Необходимо указать базовый URL медиасервера.",
  "error.reason.MEDIA_SERVER_BOTH_EMBY_ADMINISTRATOR_USERNAME_PASSWORD_REQUIRED":
    "Необходимо указать и имя пользователя, и пароль администратора Emby.",
  "error.reason.MEDIA_SERVER_BOTH_JELLYFIN_ADMIN_USERNAME_PASSWORD_REQUIRED":
    "Необходимо указать и имя пользователя, и пароль администратора Jellyfin.",
  "error.reason.MEDIA_SERVER_CANNOT_CHANGE_PROVIDER_CONNECTION_LINKED_ACCOUNTS":
    "Нельзя изменить провайдера подключения с привязанными учётными записями.",
  "error.reason.MEDIA_SERVER_CHANGING_EMBY_SERVER_URL_REQUIRES_STORED_API_KEY":
    "Для изменения URL сервера Emby необходим сохранённый ключ API.",
  "error.reason.MEDIA_SERVER_CHOOSE_EITHER_EMBY_API_KEY_LOCAL_ADMINISTRATOR_CREDENTIALS_NOT_BOTH":
    "Выберите либо ключ API Emby, либо учётные данные локального администратора, но не оба варианта.",
  "error.reason.MEDIA_SERVER_CHOOSE_EITHER_JELLYFIN_API_KEY_JELLYFIN_ADMIN_LOGIN_CREDENTIALS_NOT_BOTH":
    "Выберите либо ключ API Jellyfin, либо учётные данные администратора Jellyfin, но не оба варианта.",
  "error.reason.MEDIA_SERVER_CONNECTION_ID_REQUIRED":
    "Необходимо указать ID подключения к медиасерверу.",
  "error.reason.MEDIA_SERVER_CONNECTION_NOT_FOUND":
    "Подключение к серверу не найдено.",
  "error.reason.MEDIA_SERVER_CONNECTION_REFERENCED_LINKED_ACCOUNTS":
    "На подключение к медиасерверу ссылаются привязанные учётные записи; вместо удаления отключите его.",
  "error.reason.MEDIA_SERVER_CONNECTION_REFERENCED_NOTIFICATION_CHANNELS":
    "На подключение к медиасерверу ссылаются каналы уведомлений; вместо удаления отключите его.",
  "error.reason.MEDIA_SERVER_DISCOVER_SELECT_PLEX_SERVER_BEFORE_ENABLING_LOGIN_LINKING_AUTO_ADD":
    "Найдите и выберите сервер Plex, прежде чем включать вход, привязку или автоматическое добавление.",
  "error.reason.MEDIA_SERVER_EMBY_API_KEY_REQUIRED":
    "Необходимо указать ключ API Emby.",
  "error.reason.MEDIA_SERVER_EMBY_AVATAR_NOT_FOUND": "Аватар Emby не найден.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY":
    "У подключения Emby нет подтверждённого идентификатора сервера.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECTION_REQUIRES_VERIFIED_INTEGRATION_API_KEY":
    "Для подключения Emby необходим проверенный ключ API интеграции.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECTION_TEST_REQUIRES_SAVED_INTEGRATION_API_KEY":
    "Для проверки подключения Emby необходим сохранённый ключ API интеграции.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_LOCAL_CREDENTIALS_FORBIDDEN":
    "При настройке Emby Connect нельзя использовать вставленный ключ API или учётные данные локального администратора.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_LOGIN_REQUIRES_VERIFIED_SERVER_IDENTITY":
    "Для входа через Emby Connect необходим подтверждённый идентификатор сервера.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_PASSWORD_REQUIRED":
    "Необходимо указать пароль Emby Connect.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_SIGN_DISABLED_CONNECTION":
    "Вход через Emby Connect отключён для этого подключения.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_TEST_REQUIRES_EMBY_CONNECTION":
    "Для проверки Emby Connect необходимо подключение Emby.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_USERNAME_EMAIL_REQUIRED":
    "Необходимо указать имя пользователя или адрес электронной почты Emby Connect.",
  "error.reason.MEDIA_SERVER_EMBY_SERVER_IDENTITY_DOES_NOT_MATCH_SAVED_CONNECTION":
    "Идентификатор сервера Emby не совпадает с сохранённым подключением.",
  "error.reason.MEDIA_SERVER_EMBY_USER_LISTING_REQUIRES_EMBY_CONNECTION":
    "Для получения списка пользователей Emby необходимо подключение Emby.",
  "error.reason.MEDIA_SERVER_EMBY_USER_LISTING_REQUIRES_SAVED_INTEGRATION_API_KEY":
    "Для получения списка пользователей Emby необходим сохранённый ключ API интеграции.",
  "error.reason.MEDIA_SERVER_ENABLED_EMBY_CONNECTION_MUST_RETAIN_VERIFIED_API_KEY":
    "Во включённом подключении Emby должен сохраняться проверенный ключ API.",
  "error.reason.MEDIA_SERVER_EPISODE_NOT_FOUND": "Эпизод не найден.",
  "error.reason.MEDIA_SERVER_EXTERNAL_URL_CREDENTIALS_FORBIDDEN":
    "Внешний URL медиасервера не должен содержать учётные данные.",
  "error.reason.MEDIA_SERVER_EXTERNAL_URL_INVALID":
    "Внешний URL медиасервера должен использовать HTTP или HTTPS.",
  "error.reason.MEDIA_SERVER_EXTERNAL_URL_QUERY_OR_FRAGMENT":
    "Внешний URL медиасервера не должен содержать параметры запроса или фрагмент.",
  "error.reason.MEDIA_SERVER_JELLYFIN_USER_LISTING_REQUIRES_JELLYFIN_CONNECTION":
    "Для получения списка пользователей Jellyfin необходимо подключение Jellyfin.",
  "error.reason.MEDIA_SERVER_JELLYFIN_USER_LISTING_REQUIRES_SAVED_API_KEY":
    "Для получения списка пользователей Jellyfin необходим сохранённый ключ API; сохраните ключ API, чтобы загрузить пользователей Jellyfin.",
  "error.reason.MEDIA_SERVER_LOCAL_EMBY_SETUP_DOES_NOT_ACCEPT_EMBY_CONNECT_CREDENTIALS":
    "При локальной настройке Emby нельзя использовать учётные данные Emby Connect.",
  "error.reason.MEDIA_SERVER_PATH_MAPPINGS_INVALID":
    "Исходные пути в сопоставлениях путей медиасервера должны быть уникальными.",
  "error.reason.MEDIA_SERVER_PLEX_ACCOUNT_DOES_NOT_ACCESS_SELECTED_SERVER":
    "Учётная запись Plex не имеет доступа к выбранному серверу.",
  "error.reason.MEDIA_SERVER_PLEX_DID_NOT_RETURN_ANY_ACCESSIBLE_SERVERS_ACCOUNT":
    "Plex не вернул ни одного доступного сервера для этой учётной записи.",
  "error.reason.MEDIA_SERVER_SELECTED_EMBY_CONNECT_SERVER_IDENTITY_DOES_NOT_MATCH_REACHABLE_SERVER":
    "Идентификатор выбранного сервера Emby Connect не совпадает с доступным сервером.",
  "error.reason.MEDIA_SERVER_SELECTED_PLEX_SERVER_NOT_RETURNED_PLEX_DISCOVERY":
    "Выбранный сервер Plex отсутствует в результатах обнаружения Plex.",
  "error.reason.MEDIA_SERVER_SELECT_EMBY_CONNECT_SERVER":
    "Выберите сервер Emby Connect.",
  "error.reason.MEDIA_SERVER_SELECT_PLEX_SERVER_BEFORE_SAVING_CONNECTION":
    "Выберите сервер Plex, прежде чем сохранять это подключение.",
  "error.reason.MEDIA_SERVER_SIGN_PLEX_BEFORE_SELECTING_PLEX_SERVER":
    "Войдите через Plex, прежде чем выбирать сервер Plex.",
  "error.reason.MEDIA_SERVER_SIGN_PLEX_TEST_CONNECTION":
    "Войдите через Plex, чтобы проверить это подключение.",
  "error.reason.MEDIA_SOURCE_CHANGED_DURING_INSPECTION":
    "Источник медиа изменился во время проверки.",
  "error.reason.MEDIA_SOURCE_SELECTION_CHANGED_DURING_INSPECTION":
    "Источник медиа или выбранный элемент изменился во время проверки; обновите данные и повторите попытку.",
  "error.reason.MEDIA_SOURCE_SELECTION_CHANGED_DURING_MAPPING":
    "Источник медиа или выбранный элемент изменился во время сопоставления; обновите данные и повторите попытку.",
  "error.reason.MEDIA_TITLE_FACET_CHANGE_UNSUPPORTED":
    "Изменение типа тайтла не поддерживается, поскольку тайтлы нельзя перемещать между библиотеками.",
  "error.reason.MEDIA_TITLE_IMAGE_CONTENT_TYPE_UNSUPPORTED":
    "Неподдерживаемый тип содержимого изображения тайтла.",
  "error.reason.MEDIA_TITLE_IMAGE_HOST_UNAPPROVED":
    "Хост изображения тайтла не входит в список одобренных поставщиков изображений.",
  "error.reason.MEDIA_TITLE_IMAGE_REDIRECT_FORBIDDEN":
    "Перенаправления для изображений тайтлов запрещены.",
  "error.reason.MEDIA_TITLE_IMAGE_TOO_LARGE":
    "Изображение тайтла превышает максимальный размер в байтах.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_CREDENTIALS_FORBIDDEN":
    "URL изображения тайтла не должен содержать учётные данные.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_EMPTY":
    "URL изображения тайтла не должен быть пустым.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_HOST_REQUIRED":
    "URL изображения тайтла должен содержать хост.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_INVALID":
    "Недопустимый URL изображения тайтла.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_SCHEME_UNSUPPORTED":
    "URL изображения тайтла должен использовать http или https.",
  "error.reason.MEDIA_TITLE_MERGE_BLOCKED":
    "Выбранные тайтлы нельзя объединить.",
  "error.reason.MEDIA_TITLE_NAME_EMPTY":
    "Название тайтла не может быть пустым.",
  "error.reason.MEDIA_TITLE_SLUG_AMBIGUOUS":
    "Слаг тайтла соответствует нескольким тайтлам.",
  "error.reason.MEDIA_TITLE_TAG_LIMIT_EXCEEDED":
    "Достигнут лимит тегов тайтла.",
  "error.reason.MEDIA_TITLE_TAG_NOT_FOUND": "Тег тайтла не найден.",
  "error.reason.MEDIA_TITLE_UPDATE_EMPTY":
    "Необходимо указать хотя бы одно поле тайтла.",
  "error.reason.MEDIA_TRANSFER_DESTINATION_REQUIRED":
    "Для переноса необходимы и целевая библиотека, и целевой корневой каталог.",
  "error.reason.MEDIA_VIDEO_NOT_FOUND":
    "В выбранных медиа не найден видеопоток.",
  "error.reason.NOTIFICATION_CHANNELS_CANNOT_REFERENCE_MEDIA_SERVER_CONNECTIONS":
    "Каналы уведомлений не могут ссылаться на подключения к медиасерверам.",
  "error.reason.NOTIFICATION_CHANNEL_DISABLED": "Канал уведомлений отключён.",
  "error.reason.NOTIFICATION_CHANNEL_MEDIA_SERVER_CONNECTION_PROVIDER_DOES_NOT_MATCH_CHANNEL_TYPE":
    "Провайдер подключения к медиасерверу в канале уведомлений не соответствует типу канала.",
  "error.reason.NOTIFICATION_CHANNEL_NAME_REQUIRED":
    "Название канала не должно быть пустым.",
  "error.reason.NOTIFICATION_CHANNEL_NOT_FOUND": "Канал не найден.",
  "error.reason.NOTIFICATION_CHANNEL_TYPE_INVALID": "Недопустимый тип канала.",
  "error.reason.NOTIFICATION_CONFIGURATION_INVALID":
    "Конфигурация уведомлений должна быть объектом JSON.",
  "error.reason.NOTIFICATION_EVENT_TYPE_NOT_SUBSCRIBABLE":
    "На этот тип событий уведомлений нельзя подписаться.",
  "error.reason.NOTIFICATION_EVENT_TYPE_UNKNOWN":
    "Неизвестный тип события уведомления.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_CONNECTION_DISABLED":
    "Подключение к медиасерверу отключено.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_CONNECTION_NOT_FOUND":
    "Подключение к медиасерверу не найдено.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_NOTIFICATION_PROVIDERS_INVALID":
    "Провайдеры уведомлений медиасерверов должны адресоваться через подключения к медиасерверам.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_NOTIFICATION_TARGETS_INVALID":
    "Получателями уведомлений медиасерверов необходимо управлять в разделе «Медиасерверы».",
  "error.reason.NOTIFICATION_MEDIA_SERVER_PROVIDER_INVALID":
    "Недопустимый провайдер медиасервера: выбранный элемент.",
  "error.reason.NOTIFICATION_PLEX_MEDIA_SERVER_BASE_URL_INVALID":
    "Недопустимый базовый URL медиасервера Plex: выбранный элемент.",
  "error.reason.NOTIFICATION_PLUGIN_CHANNEL_TYPE_NOT_FOUND":
    "Плагин уведомлений для этого типа канала не найден.",
  "error.reason.NOTIFICATION_PROVIDER_DOES_NOT_SUPPORT_EVENT":
    "Выбранный провайдер уведомлений не поддерживает выбранное событие.",
  "error.reason.NOTIFICATION_PROVIDER_DOES_NOT_SUPPORT_TEST_NOTIFICATIONS":
    "Выбранный провайдер уведомлений не поддерживает тестовые уведомления.",
  "error.reason.NOTIFICATION_PROVIDER_REQUIRES":
    "Для выбранного провайдера уведомлений необходим выбранный элемент.",
  "error.reason.NOTIFICATION_PROVIDER_UNAVAILABLE":
    "Выбранный провайдер уведомлений недоступен.",
  "error.reason.NOTIFICATION_SUBSCRIPTION_NOT_FOUND": "Подписка не найдена.",
  "error.reason.NOTIFICATION_SUBSCRIPTION_TARGET_REQUIRED":
    "Необходимо указать получателя подписки на уведомления.",
  "error.reason.NOTIFICATION_TARGET_ID_REQUIRED_TARGET_KIND":
    "Для выбранного вида получателя уведомлений необходимо указать ID получателя.",
  "error.reason.NOTIFICATION_TARGET_KIND_INVALID":
    "Недопустимый вид получателя уведомлений.",
  "error.reason.NOTIFICATION_TARGET_KIND_REQUIRED_WHEN_TARGET_ID_PROVIDED":
    "При указании ID получателя необходимо указать вид получателя уведомлений.",
  "error.reason.OAUTH_AUTHLESS_AUTHORIZATION_CODE_NO_LONGER_ALLOWED":
    "Код авторизации без аутентификации больше не разрешён.",
  "error.reason.OAUTH_AUTHLESS_REFRESH_GRANT_NO_LONGER_ALLOWED":
    "Предоставление доступа по токену обновления без аутентификации больше не разрешено.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_BINDING_MISMATCH":
    "Привязка кода авторизации не совпадает.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_EXPIRED_ALREADY_USED":
    "Срок действия кода авторизации истёк или он уже использован.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_INVALID":
    "Недопустимый код авторизации.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_SESSION_UNAVAILABLE":
    "Сеанс кода авторизации больше недействителен.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_UNAVAILABLE":
    "Код авторизации больше недействителен.",
  "error.reason.OAUTH_CLIENT_DISABLED_UNAVAILABLE":
    "Клиент OAuth отключён или недоступен.",
  "error.reason.OAUTH_CLIENT_DISPLAY_NAME_REQUIRED":
    "Необходимо указать отображаемое имя клиента OAuth.",
  "error.reason.OAUTH_CLIENT_DISPLAY_NAME_TOO_LONG":
    "Длина отображаемого имени клиента OAuth не должна превышать выбранное количество символов.",
  "error.reason.OAUTH_CLIENT_NOT_FOUND": "Клиент OAuth не найден.",
  "error.reason.OAUTH_CLIENT_UNKNOWN": "Неизвестный клиент OAuth.",
  "error.reason.OAUTH_CODE_CHALLENGE_INVALID":
    "Code_challenge должен быть допустимым значением S256 base64url.",
  "error.reason.OAUTH_CODE_CHALLENGE_METHOD_INVALID":
    "Code_challenge_method должен иметь значение S256.",
  "error.reason.OAUTH_CODE_VERIFIER_INVALID":
    "Code_verifier должен содержать 43-128 незарезервированных символов RFC7636.",
  "error.reason.OAUTH_CUSTOM_OAUTH_REDIRECT_URIS_INVALID":
    "Пользовательские URI перенаправления OAuth должны быть абсолютными HTTPS URL без учётных данных.",
  "error.reason.OAUTH_GRANT_REVOKED_UNAVAILABLE":
    "Разрешение OAuth отозвано или недоступно.",
  "error.reason.OAUTH_INVALID_SCOPE":
    "Запрошенная область авторизации не поддерживается.",
  "error.reason.OAUTH_JELLYFIN_CONNECTION_AMBIGUOUS":
    "Найдено более одного подходящего подключения Jellyfin. Выберите подключение.",
  "error.reason.OAUTH_PKCE_VERIFICATION_FAILED": "Проверка PKCE не пройдена.",
  "error.reason.OAUTH_REDIRECT_URIS_INVALID":
    "URI перенаправления OAuth должны быть уникальными.",
  "error.reason.OAUTH_REDIRECT_URI_FRAGMENT_FORBIDDEN":
    "Redirect_uri не должен содержать фрагмент.",
  "error.reason.OAUTH_REDIRECT_URI_INVALID":
    "Недопустимый URI перенаправления OAuth.",
  "error.reason.OAUTH_REDIRECT_URI_NOT_ALLOWED_OAUTH_CLIENT":
    "Redirect_uri не разрешён для этого клиента OAuth.",
  "error.reason.OAUTH_REDIRECT_URI_REQUIRED":
    "Необходимо указать хотя бы один URI перенаправления OAuth.",
  "error.reason.OAUTH_REFRESH_TOKEN_CLIENT_MISMATCH":
    "Клиент токена обновления не совпадает.",
  "error.reason.OAUTH_REFRESH_TOKEN_INVALID": "Недопустимый токен обновления.",
  "error.reason.OAUTH_REFRESH_TOKEN_REUSED":
    "Токен обновления использован повторно.",
  "error.reason.OAUTH_REFRESH_TOKEN_REVOKED": "Токен обновления отозван.",
  "error.reason.OAUTH_REFRESH_TOKEN_SESSION_UNAVAILABLE":
    "Сеанс токена обновления больше недействителен.",
  "error.reason.OAUTH_TOKEN_MALFORMED": "Неверный формат токена OAuth.",
  "error.reason.OAUTH_USER_NO_LONGER_EXISTS":
    "Пользователь OAuth больше не существует.",
  "error.reason.PLUGIN_ALREADY_INSTALLED": "Этот плагин уже установлен.",
  "error.reason.PLUGIN_ALREADY_VERSION_SELECTED_RELEASE":
    "У выбранного плагина уже установлена выбранная версия.",
  "error.reason.PLUGIN_ARCHIVE_EXTRACTOR_CONFLICT":
    "Этот формат поддерживают несколько включённых плагинов распаковки архивов. Отключите один из них в разделе «Настройки → Плагины».",
  "error.reason.PLUGIN_ARCHIVE_PATH_OUTSIDE_ROOT":
    "Путь плагина в архиве находится за пределами разрешённого корневого каталога.",
  "error.reason.PLUGIN_ARCHIVE_PATH_UNSAFE":
    "Путь плагина в архиве не является безопасным относительным путём.",
  "error.reason.PLUGIN_ARTIFACT_DIGEST_REQUIRED":
    "Запрошенное значение должно содержать хотя бы один хеш.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECTS_NOT_ALLOWED":
    "Перенаправления артефакта плагина запрещены для выбранного значения.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_DID_NOT_INCLUDE_LOCATION_HEADER":
    "Перенаправление артефакта плагина для выбранного значения не содержит заголовка Location.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_HEADER":
    "Перенаправление артефакта плагина для выбранного значения содержит недопустимый заголовок Location.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_URL":
    "Перенаправление артефакта плагина для выбранного значения содержит недопустимый URL в Location.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_LIMIT_EXCEEDED":
    "Превышен лимит перенаправлений артефакта плагина для выбранного значения.",
  "error.reason.PLUGIN_ARTIFACT_UNTRUSTED":
    "Не удалось подтвердить доверие к артефакту плагина.",
  "error.reason.PLUGIN_BLOCKED_VERSION_SCRYER_SO_CANNOT_TESTED":
    "Выбранный плагин заблокирован в этой версии Scryer, поэтому его нельзя проверить. Выбранное значение.",
  "error.reason.PLUGIN_BROTLI_DECOMPRESSION_NOT_COMPILED_INTO_TARGET":
    "Распаковка плагинов brotli не включена в эту сборку.",
  "error.reason.PLUGIN_BUNDLED_PLUGIN":
    "Выбранный плагин входит в комплект поставки; удалите загруженную замену, прежде чем загружать локальную сборку.",
  "error.reason.PLUGIN_CANNOT_REVERT_BUILT_PLUGIN_NO_BUNDLED_DEFINITION_UNAVAILABLE":
    "Нельзя восстановить встроенный плагин: для выбранного элемента недоступно определение из комплекта поставки.",
  "error.reason.PLUGIN_CANNOT_UNINSTALL_BUILT_PLUGINS":
    "Встроенные плагины нельзя удалить; вместо этого отключите их.",
  "error.reason.PLUGIN_CATALOG_ARTIFACTS_MISSING":
    "Перенаправление каталога плагинов не содержит артефактов.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_CATALOG_VERSION_OUT_OF_RANGE":
    "Значение catalog_version перенаправления каталога плагинов должно быть больше нуля.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_DID_NOT_YIELD_USABLE_CATALOG":
    "Перенаправление каталога плагинов не привело к получению пригодного каталога.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_JSON_INVALID":
    "Недопустимый JSON перенаправления каталога плагинов.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_OUT_OF_RANGE":
    "Перенаправление каталога плагинов должно содержать хотя бы один артефакт.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_SCHEMA_UNSUPPORTED":
    "Неподдерживаемая схема перенаправления каталога плагинов: выбранный элемент.",
  "error.reason.PLUGIN_CATALOG_SCHEMA_UNSUPPORTED":
    "Неподдерживаемая схема каталога плагинов: выбранный элемент.",
  "error.reason.PLUGIN_CATALOG_V3_JSON_INVALID":
    "Недопустимый JSON каталога плагинов v3.",
  "error.reason.PLUGIN_CATALOG_VERSION_INVALID":
    "Недопустимая версия каталога: выбранный элемент.",
  "error.reason.PLUGIN_CATALOG_VERSION_OUT_OF_RANGE":
    "Версия каталога плагинов должна быть больше нуля.",
  "error.reason.PLUGIN_CHANGED_WHILE_RUNTIME_BEING_PREPARED":
    "Плагин изменился во время подготовки его среды выполнения.",
  "error.reason.PLUGIN_CLIENT_CONFIG_JSON_INVALID":
    "Недопустимый config_json клиента.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_APPROVED_TIER_INVALID":
    "Одобренный уровень источника каталога сообщества для выбранного элемента должен быть verified_community.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_MUST_PUBLISH_EXACTLY_ONE_PLUGIN":
    "Выбранный источник каталога сообщества должен публиковать ровно один плагин.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_PUBLISHED_PLUGIN":
    "Выбранный источник каталога сообщества опубликовал выбранный плагин.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_SIGNER_REPO_DOES_NOT_MATCH_APPROVED_REPO":
    "Репозиторий подписанта выбранного источника каталога сообщества не совпадает с выбранным одобренным репозиторием.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_SOURCE_REPO_DOES_NOT_MATCH_APPROVED_REPO":
    "Репозиторий исходного кода выбранного источника каталога сообщества не совпадает с выбранным одобренным репозиторием.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_SUPPORT_TIER_DOES_NOT_MATCH_APPROVED_TIER":
    "Уровень поддержки выбранного источника каталога сообщества не совпадает с одобренным уровнем.",
  "error.reason.PLUGIN_COMMUNITY_RULE_PACK_PUBLISH_FORBIDDEN":
    "Выбранный источник каталога сообщества не должен публиковать пакеты правил.",
  "error.reason.PLUGIN_COMMUNITY_SOURCE_ID_PLUGIN_CATALOG_DUPLICATED":
    "Повторяющийся ID выбранного источника сообщества в каталоге плагинов.",
  "error.reason.PLUGIN_COMMUNITY_SOURCE_NESTING_FORBIDDEN":
    "Выбранный источник каталога сообщества не должен публиковать вложенные источники сообщества.",
  "error.reason.PLUGIN_COMMUNITY_SOURCE_SUPPORT_TIER_INVALID":
    "Уровень поддержки выбранного источника сообщества должен быть verified_community.",
  "error.reason.PLUGIN_COMPONENT_DESCRIPTOR_DIFFERS_PERSISTED_METADATA":
    "Дескриптор компонента отличается от сохранённых метаданных.",
  "error.reason.PLUGIN_COMPONENT_DESCRIPTOR_MISMATCH":
    "Дескриптор инициализированного компонента отличается от встроенных метаданных.",
  "error.reason.PLUGIN_COMPONENT_REPLACEMENT_CHANGED_PLUGIN_IDENTITY":
    "Замена компонента изменила идентификатор плагина.",
  "error.reason.PLUGIN_COMPONENT_UPGRADE_BLOCKED":
    "Обновление плагина заблокировано установленными компонентами.",
  "error.reason.PLUGIN_DECOMPRESSED_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES":
    "Распакованные данные запрошенного значения превышают максимальный размер — выбранное количество байт.",
  "error.reason.PLUGIN_DECOMPRESSED_SIZE_LIMIT_EXCEEDS_PLATFORM_S_MAXIMUM_BUFFER_SIZE":
    "Лимит размера распакованных данных запрошенного значения, равный выбранному значению, превышает максимальный размер буфера этой платформы.",
  "error.reason.PLUGIN_DESCRIPTOR_ID_MISMATCH":
    "ID дескриптора загруженного плагина не совпадает с выбранным ID в реестре.",
  "error.reason.PLUGIN_DIGEST_ALGORITHM_REQUIRED":
    "Не указан алгоритм хеширования.",
  "error.reason.PLUGIN_DIGEST_MISMATCH":
    "Хеш запрошенного значения не совпадает: ожидался blake3.",
  "error.reason.PLUGIN_DIGEST_STRING_INVALID":
    "Недопустимая строка хеша: выбранный элемент.",
  "error.reason.PLUGIN_DIGEST_VALUE_INVALID":
    "Выбранное значение хеша должно быть шестнадцатеричным.",
  "error.reason.PLUGIN_DIGEST_VALUE_REQUIRED": "Не указано значение хеша.",
  "error.reason.PLUGIN_DOES_NOT_INCLUDE_BLAKE3_DIGEST":
    "Запрошенное значение не содержит хеш blake3.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_PLUGIN_TYPE_BUT_REGISTRY_EXPECTS":
    "Значение plugin_type загруженного выбранного плагина не совпадает со значением, ожидаемым реестром.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_PROVIDER_TYPE_BUT_REGISTRY_EXPECTS":
    "Значение provider_type загруженного выбранного плагина не совпадает со значением, ожидаемым реестром.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_SDK_VERSION_BUT_SELECTED_RELEASE":
    "Значение sdk_version загруженного выбранного плагина не совпадает со значением выбранного релиза.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_VERSION_BUT_SELECTED_RELEASE":
    "Версия загруженного выбранного плагина не совпадает с версией выбранного релиза.",
  "error.reason.PLUGIN_FIELD_REQUIRED":
    "Необходимо указать запрошенное значение.",
  "error.reason.PLUGIN_GITHUB_OWNER_REQUIRED": "Не указан владелец GitHub.",
  "error.reason.PLUGIN_GITHUB_REPOSITORY_INVALID":
    "Репозиторий GitHub должен включать владельца и имя репозитория.",
  "error.reason.PLUGIN_GITHUB_REPOSITORY_URL_INVALID":
    "Недопустимый URL репозитория GitHub.",
  "error.reason.PLUGIN_GITHUB_REPO_NAME_REQUIRED":
    "Не указано имя репозитория GitHub.",
  "error.reason.PLUGIN_ID_PLUGIN_CATALOG_DUPLICATED":
    "Повторяющийся ID выбранного плагина в каталоге плагинов.",
  "error.reason.PLUGIN_INDEXER_HOST_FORBIDDEN":
    "Хост индексатора не разрешён для этого экземпляра.",
  "error.reason.PLUGIN_INSTALLED_PACK_VERSION_INVALID":
    "Недопустимая версия установленного пакета.",
  "error.reason.PLUGIN_INSTALLED_PLUGIN_SETTINGS_DESCRIPTOR_UNAVAILABLE":
    "Дескриптор настроек установленного плагина недоступен.",
  "error.reason.PLUGIN_INSTALLED_VERSION_INVALID":
    "Недопустимая установленная версия: выбранный элемент.",
  "error.reason.PLUGIN_INVALID_PERSISTED_DESCRIPTOR_JSON":
    "У выбранного плагина недопустимый сохранённый descriptor_json.",
  "error.reason.PLUGIN_LIST_LINKED_ACCOUNT_REQUIRED":
    "Провайдер списков читает только собственные списки участника и требует привязанную учётную запись этого участника.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_CATALOG_NOT_UTF_8":
    "Каталог плагинов для ручной установки имеет кодировку, отличную от UTF-8.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_CATALOG_SOURCE_MISSING_GITHUB_REPO":
    "У выбранного источника каталога плагинов для ручной установки не указан репозиторий GitHub.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_MISSING_SOURCE_REPO":
    "У выбранного плагина для ручной установки не указан репозиторий исходного кода.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPOSITORIES_INVALID":
    "Репозитории плагинов для ручной установки должны размещаться на github.com.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPO_MUST_PUBLISH_EXACTLY_ONE_PLUGIN_ENTRY":
    "Выбранный репозиторий плагина для ручной установки должен публиковать ровно одну запись плагина.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPO_NO_SDK_COMPATIBLE_RELEASE":
    "В выбранном репозитории плагина для ручной установки нет релиза, совместимого с SDK.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPO_PUBLISHED_PLUGIN_SOURCE_REPO":
    "Выбранный репозиторий плагина для ручной установки опубликовал выбранный плагин из выбранного репозитория исходного кода.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_FILE_NAME_REQUIRED":
    "Необходимо указать имя загружаемого файла плагина для ручной установки.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_INVALID":
    "Для ручной установки плагина необходимо загрузить файл .wasm или .wasm.zst.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_PAYLOAD_INVALID":
    "Данные загружаемого плагина для ручной установки имеют недопустимый формат base64.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_REQUIRES_EXPLICIT_RISK_ACKNOWLEDGEMENT":
    "Ручная загрузка плагина требует явного подтверждения рисков.",
  "error.reason.PLUGIN_MISSING_PERSISTED_DESCRIPTOR_JSON":
    "У выбранного плагина отсутствует сохранённый descriptor_json.",
  "error.reason.PLUGIN_MISSING_PERSISTED_WASM_DIGEST_ALGORITHM":
    "У выбранного плагина отсутствует сохранённый алгоритм хеширования wasm.",
  "error.reason.PLUGIN_MISSING_PERSISTED_WASM_DIGEST_VALUE":
    "У выбранного плагина отсутствует сохранённое значение хеша wasm.",
  "error.reason.PLUGIN_MISSING_PERSISTED_WASM_PAYLOAD":
    "У выбранного плагина отсутствуют сохранённые данные WASM.",
  "error.reason.PLUGIN_MISSING_USABLE_BLAKE3_DIGEST":
    "У запрошенного значения отсутствует пригодный хеш blake3.",
  "error.reason.PLUGIN_MUST_USE_SUPPORTED_ALGORITHM_HEX_DIGEST":
    "Для запрошенного значения необходимо использовать поддерживаемый хеш в формате <algorithm>:<hex>.",
  "error.reason.PLUGIN_NETWORK_ENDPOINT_RESOLUTION_FAILED":
    "Не удалось разрешить адреса настроенных конечных точек индексатора.",
  "error.reason.PLUGIN_NEWZNAB_PROFILE_INVALID":
    "Выбранный профиль провайдера Newznab недопустим.",
  "error.reason.PLUGIN_NOT_AVAILABLE_PLUGIN_CATALOG_NOT_FOUND":
    "Плагин не найден в каталоге плагинов и недоступен для установки.",
  "error.reason.PLUGIN_NO_LONGER_HOST_COMPATIBLE_RELEASE_SCRYER_VERSION":
    "Для выбранного плагина больше нет релиза, совместимого с этой версией Scryer.",
  "error.reason.PLUGIN_NO_LONGER_INSTALLED_NOT_FOUND":
    "Плагин не найден: он больше не установлен.",
  "error.reason.PLUGIN_OFFICIAL_NZBGEEK_PLUGIN_NOT_AVAILABLE_BUILTIN_MIGRATION_NOT_FOUND":
    "Официальный плагин nzbgeek недоступен для миграции встроенного плагина.",
  "error.reason.PLUGIN_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES":
    "Данные запрошенного значения превышают максимальный размер — выбранное количество байт.",
  "error.reason.PLUGIN_PAYLOAD_TOO_LARGE_VALIDATE_DECOMPRESSED_SIZE":
    "Данные запрошенного значения слишком велики для проверки размера после распаковки.",
  "error.reason.PLUGIN_PROVIDER_TYPE_RESERVED_FIRST_PARTY_CODE":
    "Выбранный тип провайдера зарезервирован для собственного кода приложения.",
  "error.reason.PLUGIN_PROXY_CONFIGURATION_NOT_FOUND":
    "Конфигурация прокси не найдена.",
  "error.reason.PLUGIN_PROXY_DISABLED_DOWNLOAD_CLIENT":
    "Прокси отключён для этого клиента загрузки.",
  "error.reason.PLUGIN_RELEASE_ARTIFACT_BYTES_OUT_OF_RANGE":
    "Размер в байтах выбранного артефакта выбранного релиза выбранного плагина должен быть больше нуля.",
  "error.reason.PLUGIN_RELEASE_ARTIFACT_INVALID":
    "Выбранный артефакт выбранного релиза выбранного плагина должен содержать wasm_digests.",
  "error.reason.PLUGIN_RELEASE_DUPLICATE_ARTIFACT_VARIANT_ENCODING_ROWS":
    "Выбранный релиз выбранного плагина содержит повторяющиеся строки варианта/кодировки артефакта.",
  "error.reason.PLUGIN_RELEASE_INVALID_MAX_SCRYER_VERSION":
    "В выбранном релизе выбранного плагина указано недопустимое значение max_scryer_version.",
  "error.reason.PLUGIN_RELEASE_INVALID_MIN_SCRYER_VERSION":
    "В выбранном релизе выбранного плагина указано недопустимое значение min_scryer_version.",
  "error.reason.PLUGIN_RELEASE_MIN_SCRYER_VERSION_GREATER_MAX_SCRYER_VERSION":
    "В выбранном релизе выбранного плагина значение min_scryer_version больше max_scryer_version.",
  "error.reason.PLUGIN_RELEASE_OUT_OF_RANGE":
    "Выбранный релиз выбранного элемента должен содержать хотя бы один артефакт.",
  "error.reason.PLUGIN_RELEASE_VERSION_INVALID":
    "Недопустимая версия релиза плагина: выбранный элемент.",
  "error.reason.PLUGIN_RELEASE_VERSION_PLUGIN_DUPLICATED":
    "Повторяющаяся выбранная версия релиза для выбранного плагина.",
  "error.reason.PLUGIN_REQUESTS_HOST_PROCESS_CAPABILITY_WHICH_RESERVED_OFFICIAL_PLUGINS":
    "Выбранный плагин запрашивает возможность host-process, зарезервированную для официальных плагинов.",
  "error.reason.PLUGIN_REQUIRED_PLUGIN_SETTING_REQUIRED":
    "Отсутствует обязательная настройка плагина.",
  "error.reason.PLUGIN_RESOURCE_NOT_FOUND":
    "Плагин не найден: он не установлен.",
  "error.reason.PLUGIN_RULE_PACK_CATALOG_CHANGED_DURING_DOWNLOAD":
    "Каталог пакетов правил изменился во время загрузки.",
  "error.reason.PLUGIN_RULE_PACK_CONTAINS_DUPLICATE_RULE_ID":
    "Выбранный пакет правил содержит повторяющийся ID выбранного правила.",
  "error.reason.PLUGIN_RULE_PACK_CONTAINS_NO_RULES":
    "Выбранный пакет правил не содержит правил.",
  "error.reason.PLUGIN_RULE_PACK_CONTAINS_RULE_REQUIRED_FIELDS_MISSING":
    "Выбранный пакет правил содержит правило без обязательных полей.",
  "error.reason.PLUGIN_RULE_PACK_DOES_NOT_MATCH_CATALOG_ID_VERSION":
    "ID или версия выбранного пакета правил не совпадает с каталогом.",
  "error.reason.PLUGIN_RULE_PACK_EMPTY":
    "В выбранном пакете правил выбранное значение пусто.",
  "error.reason.PLUGIN_RULE_PACK_ID_PLUGIN_CATALOG_DUPLICATED":
    "Повторяющийся ID выбранного пакета правил в каталоге плагинов.",
  "error.reason.PLUGIN_RULE_PACK_INVALID_MIN_SCRYER_VERSION":
    "В выбранном пакете правил указано недопустимое выбранное значение min_scryer_version.",
  "error.reason.PLUGIN_RULE_PACK_NOT_FOUND": "Пакет правил не найден.",
  "error.reason.PLUGIN_RULE_PACK_NO_COMPATIBLE_ARTIFACT":
    "Для выбранного пакета правил нет совместимого артефакта.",
  "error.reason.PLUGIN_RULE_PACK_RELEASE_INVALID":
    "Выбранный релиз выбранного пакета правил должен содержать rule_pack_digests.",
  "error.reason.PLUGIN_RULE_PACK_RELEASE_VERSION_DUPLICATED":
    "Повторяющаяся выбранная версия релиза для выбранного пакета правил.",
  "error.reason.PLUGIN_RULE_PACK_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING":
    "Выбранный артефакт выбранного пакета правил имеет неподдерживаемую кодировку.",
  "error.reason.PLUGIN_RULE_PACK_USES_UNSUPPORTED_SCHEMA_VERSION":
    "Выбранный пакет правил использует неподдерживаемую выбранную версию схемы.",
  "error.reason.PLUGIN_SDK_CONSTRAINT_PLUGIN_INVALID":
    "Недопустимое выбранное значение sdk_constraint для выбранного плагина.",
  "error.reason.PLUGIN_SELECTED_ARTIFACT_BYTES_MISMATCH":
    "Размер в байтах выбранного артефакта выбранного плагина не совпадает: ожидалось выбранное значение, получено выбранное значение.",
  "error.reason.PLUGIN_SELECTED_ARTIFACT_TOO_LARGE_VALIDATE_BYTES":
    "Выбранный артефакт выбранного плагина слишком велик для проверки размера в байтах.",
  "error.reason.PLUGIN_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING":
    "Выбранный артефакт выбранного плагина имеет неподдерживаемую кодировку.",
  "error.reason.PLUGIN_SETTING_UNKNOWN": "Неизвестная настройка плагина.",
  "error.reason.PLUGIN_SETTING_VALUE_INVALID":
    "Недопустимое значение настройки плагина.",
  "error.reason.PLUGIN_SIGNATURE_VERIFICATION_NOT_COMPILED_INTO_TARGET":
    "Проверка подписей плагинов не включена в эту сборку.",
  "error.reason.PLUGIN_SIGNER_REPO_DOES_NOT_MATCH_SOURCE_REPO":
    "Выбранный репозиторий подписанта выбранного плагина не совпадает с выбранным репозиторием исходного кода.",
  "error.reason.PLUGIN_SUBTITLE_CONFIG_NOT_OBJECT":
    "Значение config_json провайдера субтитров должно быть объектом JSON.",
  "error.reason.PLUGIN_SUBTITLE_HOST_BINDING_REQUIRED":
    "У провайдера субтитров отсутствует обязательная привязка к хосту.",
  "error.reason.PLUGIN_SUBTITLE_INPUT_FILENAME_REQUIRED":
    "Входной путь для субтитров должен указывать на файл.",
  "error.reason.PLUGIN_SUBTITLE_INPUT_TOO_LARGE":
    "Входные данные для субтитров превышают ограничение размера.",
  "error.reason.PLUGIN_SUBTITLE_INPUT_TOO_LONG":
    "Входные данные для субтитров превышают ограничение продолжительности.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_FILENAME_REQUIRED":
    "Путь для синхронизации субтитров должен указывать на файл.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_NOT_FILE":
    "Результат работы плагина синхронизации субтитров не является обычным файлом.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_OUTSIDE_DIRECTORY":
    "Результат работы плагина синхронизации субтитров находится за пределами выходного каталога.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_PATH_MISMATCH":
    "Плагин синхронизации субтитров вернул неожиданный выходной путь.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_TOO_LARGE":
    "Результат синхронизации субтитров превышает ограничение размера.",
  "error.reason.PLUGIN_TEST_CONNECTION_UNSUPPORTED":
    "Проверка подключения не поддерживается для выбранного типа клиента.",
  "error.reason.PLUGIN_TYPE_BUILT_PREPARATION_UNSUPPORTED":
    "Выбранный plugin_type не поддерживается при подготовке встроенного плагина.",
  "error.reason.PLUGIN_TYPE_BUILT_RESTORE_UNSUPPORTED":
    "Выбранный plugin_type не поддерживается при восстановлении встроенного плагина.",
  "error.reason.PLUGIN_TYPE_RUNTIME_REMOVAL_UNSUPPORTED":
    "Выбранный plugin_type не поддерживается при удалении из среды выполнения.",
  "error.reason.PLUGIN_TYPE_RUNTIME_UPSERT_UNSUPPORTED":
    "Выбранный plugin_type не поддерживается при добавлении или обновлении в среде выполнения.",
  "error.reason.PLUGIN_URL_INVALID":
    "Выбранный URL запрошенного значения должен использовать http или https; получен выбранный элемент.",
  "error.reason.PLUGIN_USES_UNSUPPORTED_DIGEST_ALGORITHM":
    "Запрошенное значение использует выбранный неподдерживаемый алгоритм хеширования.",
  "error.reason.PLUGIN_WASM_ARTIFACT_TOO_LARGE":
    "Заявленный размер WASM-артефакта выбранного плагина, равный выбранному значению, превышает максимальный размер — выбранное количество байт.",
  "error.reason.PLUGIN_ZSTD_COMPRESSION_NOT_COMPILED_INTO_TARGET":
    "Сжатие плагинов zstd не включено в эту сборку.",
  "error.reason.PLUGIN_ZSTD_DECOMPRESSION_NOT_COMPILED_INTO_TARGET":
    "Распаковка плагинов zstd не включена в эту сборку.",
  "error.reason.PORTS_ARTIFACT_DOWNLOAD_UNSUPPORTED":
    "Этот клиент не поддерживает загрузку артефактов.",
  "error.reason.PORTS_CACHED_JPEG_ENCODING_UNAVAILABLE":
    "Кодирование кешированных изображений JPEG недоступно.",
  "error.reason.PORTS_CATEGORY_OUT_OF_RANGE":
    "Категория должна занимать не более 255 байт и не содержать управляющих символов.",
  "error.reason.PORTS_CLIENT_ROUTING_DISCOVERY_UNSUPPORTED":
    "Обнаружение маршрутизации клиента не поддерживается.",
  "error.reason.PORTS_DISC_TITLE_SELECTION_UNSUPPORTED_ANALYZER":
    "Этот анализатор не поддерживает выбор тайтла диска.",
  "error.reason.PORTS_INDEXER_CONFIGURATION_NOT_FOUND":
    "Конфигурация индексатора не найдена.",
  "error.reason.PORTS_INDEXER_SETTINGS_CHANGED_DURING_VALIDATION":
    "Настройки индексатора изменились во время проверки; перезагрузите данные и повторите попытку.",
  "error.reason.PORTS_MULTIPLE_TITLES_FOUND_SLUG_LOOKUP":
    "При поиске по слагу найдено несколько тайтлов.",
  "error.reason.PORTS_SELECT_DOWNLOAD_CLIENT": "Выберите клиент загрузки.",
  "error.reason.PORTS_STRUCTURAL_DIAGNOSTICS_UNSUPPORTED_ANALYZER":
    "Этот анализатор не поддерживает структурную диагностику.",
  "error.reason.QUALITY_FILE_NOT_FOUND": "Выбранный файл не найден.",
  "error.reason.QUALITY_FILE_SCORE_EVALUATION_FAILED":
    "Не удалось вычислить оценку файла; предыдущая оценка сохранена.",
  "error.reason.QUALITY_FILE_SCORING_INPUTS_CHANGED":
    "Входные данные для оценки файла изменились; обновите ещё раз.",
  "error.reason.QUALITY_MEDIA_FILE_NOT_FOUND": "Медиафайл не найден.",
  "error.reason.QUALITY_MEDIA_TITLE_NOT_FOUND": "Медиатайтл не найден.",
  "error.reason.QUALITY_SCORE_REFRESH_BELONGS_ANOTHER_USER":
    "Обновление оценки принадлежит другому пользователю.",
  "error.reason.QUALITY_SCORE_REFRESH_QUEUE_FULL":
    "Очередь обновления оценок заполнена.",
  "error.reason.QUALITY_SCORE_REFRESH_USER_NO_LONGER_EXISTS":
    "Пользователь, запустивший обновление оценки, больше не существует.",
  "error.reason.QUALITY_SEASON_NOT_FOUND": "Выбранный сезон не найден.",
  "error.reason.QUALITY_SELECT_BETWEEN_1_1000_FILES_SEASONS_TITLES":
    "Выберите от 1 до 1000 файлов, сезонов или тайтлов.",
  "error.reason.QUALITY_TITLE_NOT_FOUND": "Выбранный тайтл не найден.",
  "error.reason.RELEASE_TOKEN_EXPIRED":
    "Срок действия результатов поиска истёк. Выполните поиск снова.",
  "error.reason.RELEASE_TOKEN_INVALID":
    "Не удалось проверить выбор релиза. Выполните поиск снова.",
  "error.reason.REQUEST_RULE_FAILED_BUILD_REQUEST_RULES_ENGINE":
    "Не удалось создать движок правил запросов.",
  "error.reason.REQUEST_RULE_FIELD_REQUIRED":
    "Запрошенное значение не должно быть пустым.",
  "error.reason.REQUEST_RULE_LEASE_OUT_OF_RANGE":
    "Срок действия должен быть не больше выбранного количества дней (десять лет) либо бессрочным.",
  "error.reason.REQUEST_RULE_LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET":
    "Тип библиотеки не соответствует типу запрошенных медиа.",
  "error.reason.REQUEST_RULE_LIBRARY_NOT_FOUND": "Библиотека не найдена.",
  "error.reason.REQUEST_RULE_MAY_NOT_READ_REQUEST_S_DECISION":
    "У вас нет права просматривать решение по этому запросу.",
  "error.reason.REQUEST_RULE_MEDIA_REQUESTS_INVALID":
    "Запросы медиа должны содержать внешние идентификаторы SMG.",
  "error.reason.REQUEST_RULE_MEDIA_REQUEST_ID_REQUIRED":
    "Необходимо указать ID запроса медиа.",
  "error.reason.REQUEST_RULE_MEDIA_REQUEST_NOT_FOUND":
    "Запрос медиа не найден.",
  "error.reason.REQUEST_RULE_PERSON_TARGETING_PERMISSION_REQUIRED":
    "Это правило использует сведения о конкретных людях, поэтому для его сохранения нужно право управления разрешениями. Попросите администратора сохранить его или перепишите правило без этих сведений.",
  "error.reason.REQUEST_RULE_REQUEST_MAY_CARRY_MOST_TAGS_SUPPLIED":
    "Запрос может содержать не более выбранного количества тегов; передано выбранное количество.",
  "error.reason.REQUEST_RULE_RULE_EVALUATION_FAILED":
    "Не удалось выполнить правило.",
  "error.reason.REQUEST_RULE_RULE_FAILED_COMPILE":
    "Не удалось скомпилировать правило.",
  "error.reason.REQUEST_RULE_RULE_VALIDATION_FAILED":
    "Проверка правила не пройдена.",
  "error.reason.REQUEST_RULE_SAMPLE_LIBRARY_NOT_FOUND":
    "Библиотека для примера не найдена.",
  "error.reason.REQUEST_RULE_SAMPLE_REQUESTER_NOT_FOUND":
    "Автор запроса для примера не найден.",
  "error.reason.REQUEST_RULE_SET_NOT_FOUND": "Набор правил запросов не найден.",
  "error.reason.REQUEST_RULE_TAG_CHARACTERS_MOST_ALLOWED":
    "Тег содержит выбранное количество символов; допускается не более выбранного количества.",
  "error.reason.REQUEST_RULE_TAG_CONTAINS_UNSUPPORTED_CHARACTER":
    "Выбранный тег содержит выбранный неподдерживаемый символ; используйте буквы, цифры, '.', '_', '-' или пробелы.",
  "error.reason.REQUEST_RULE_TAG_REQUIRED": "Тег не должен быть пустым.",
  "error.reason.REQUEST_RULE_TAG_USES_RESERVED_PREFIX":
    "Выбранный тег использует выбранный зарезервированный префикс.",
  "error.reason.RULE_AUTOMATIC_RULE_PACK_UPDATES_REQUIRE_STABLE_PATCH_INSTALLED_MAJOR_MINOR":
    "Для автоматического обновления пакетов правил требуется стабильное исправление в рамках установленной основной и дополнительной версии.",
  "error.reason.RULE_BUNDLED_RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE":
    "Установка пакета правил из комплекта поставки вступила в конфликт с другим изменением.",
  "error.reason.RULE_BUNDLED_TRASH_PACK_CONTAINS_NO_TEMPLATES":
    "Пакет TRaSH из комплекта поставки не содержит шаблонов.",
  "error.reason.RULE_BUNDLED_TRASH_PACK_INVALID":
    "Недопустимый пакет TRaSH из комплекта поставки.",
  "error.reason.RULE_BUNDLED_TRASH_PACK_UNSUPPORTED_IDENTITY_SCHEMA":
    "Пакет TRaSH из комплекта поставки имеет неподдерживаемый идентификатор или схему.",
  "error.reason.RULE_CANNOT_ADOPT_DUPLICATE_LEGACY_LOCALE_RULES":
    "Нельзя принять в управление повторяющиеся устаревшие правила локали для выбранного значения.",
  "error.reason.RULE_COPYING_TRACKED_RULE_REQUIRES_DISABLING_SOURCE":
    "Для копирования отслеживаемого правила необходимо отключить его источник.",
  "error.reason.RULE_COPY_DISABLES_SOURCE_REQUIRES_COPY_SOURCE_RULE_SET":
    "Copy_disables_source требует указать исходный набор правил для копирования.",
  "error.reason.RULE_ENABLED_TEMPLATES_INVALID":
    "Включённые шаблоны должны быть уникальными элементами выбранного пакета.",
  "error.reason.RULE_EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE":
    "При проверке правила для фильма нельзя выбирать эпизод.",
  "error.reason.RULE_EPISODE_DOES_NOT_BELONG_TITLE":
    "Эпизод с выбранным значением не относится к тайтлу с выбранным значением.",
  "error.reason.RULE_EPISODE_NOT_FOUND": "Эпизод не найден.",
  "error.reason.RULE_EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE":
    "Для проверки правила сериала или аниме необходимо указать эпизод.",
  "error.reason.RULE_FAILED_BUILD_RULES_ENGINE":
    "Не удалось создать движок правил.",
  "error.reason.RULE_INDEXER_LANGUAGES_TOO_LARGE_SCORING_PREVIEW":
    "Данные языков индексатора слишком велики для предварительного расчёта оценки.",
  "error.reason.RULE_INPUT_RETIRED":
    "Правило использует выведенное из эксплуатации поле входных данных.",
  "error.reason.RULE_MANAGED_CONVENIENCE_SETTING":
    "Это правило управляется упрощающей настройкой. Измените настройку вместо редактирования самого правила.",
  "error.reason.RULE_MANAGED_LOCALE_FILTER_FORBIDDEN":
    "Управляемые пакеты локалей TRaSH Guides используют предопределённую политику локали и не допускают фильтрации.",
  "error.reason.RULE_MANAGED_TRACKED_RULES_INVALID":
    "Управляемые или отслеживаемые правила необходимо скопировать перед редактированием.",
  "error.reason.RULE_MEDIA_FILE_DOES_NOT_COVER_EPISODE":
    "Медиафайл с выбранным значением не содержит эпизод с выбранным значением.",
  "error.reason.RULE_MEDIA_FILE_NOT_FOUND": "Медиафайл тайтла не найден.",
  "error.reason.RULE_ONLY_ONE_ENABLED_RULE_MAY_USE_EXCLUSIVE_GROUP":
    "Только одно включённое правило может использовать выбранную группу взаимоисключения.",
  "error.reason.RULE_PACK_ALREADY_INSTALLED":
    "Выбранный пакет правил уже установлен.",
  "error.reason.RULE_PACK_FACET_INVALID":
    "Недопустимый тип пакета правил с выбранным значением.",
  "error.reason.RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE":
    "Установка пакета правил вступила в конфликт с другим изменением.",
  "error.reason.RULE_PACK_PRIORITY_TEMPLATE_ID_DUPLICATED":
    "Повторяющийся ID шаблона приоритета пакета правил.",
  "error.reason.RULE_PACK_SETTINGS_REFERENCE_UNKNOWN_REMOVED_TEMPLATE":
    "Настройки пакета правил ссылаются на неизвестный или удалённый шаблон.",
  "error.reason.RULE_PACK_TEMPLATE_ID_DUPLICATED":
    "Повторяющийся ID шаблона пакета правил.",
  "error.reason.RULE_PACK_UPDATE_INVALID":
    "Обновление пакета правил должно иметь более новую версию.",
  "error.reason.RULE_PACK_UP_TO_DATE":
    "Пакет правил уже обновлён до актуальной версии.",
  "error.reason.RULE_PACK_VERSION_DIGEST_DID_NOT_MATCH_VERIFIED_CATALOG_RELEASE":
    "Версия или хеш пакета правил не совпадает с проверенным релизом каталога.",
  "error.reason.RULE_PREVIEW_CANNOT_EDIT_COPY_RULE_SAME_TIME":
    "В режиме предварительного просмотра нельзя одновременно редактировать и копировать правило.",
  "error.reason.RULE_PREVIEW_REQUIRES_EITHER_DRAFT_SAVED_RULE_SET":
    "Для предварительного просмотра необходим либо черновик, либо сохранённый набор правил.",
  "error.reason.RULE_PUBLISHED_INVALID":
    "Дата публикации должна быть временной меткой RFC 2822 или RFC 3339; получено выбранное значение.",
  "error.reason.RULE_RELEASE_NAME_TOO_LARGE_SCORING_PREVIEW":
    "Название релиза слишком велико для предварительного расчёта оценки.",
  "error.reason.RULE_RULES_PACK_CANNOT_COPIED":
    "Правила из этого пакета нельзя копировать.",
  "error.reason.RULE_SET_FIELD_REQUIRED":
    "Необходимо указать хотя бы одно поле набора правил.",
  "error.reason.RULE_SET_NOT_FOUND": "Набор правил не найден.",
  "error.reason.RULE_SET_NOT_TRACKED_RULE_PACK":
    "Набор правил не отслеживается пакетом правил.",
  "error.reason.RULE_SIZE_BYTES_CANNOT_NEGATIVE":
    "Размер в байтах не может быть отрицательным.",
  "error.reason.RULE_SOURCE_INVALID": "Недопустимый исходный код правила.",
  "error.reason.RULE_SOURCE_USES_RETIRED_FIELDS":
    "Исходный код правила использует выведенные из эксплуатации поля.",
  "error.reason.RULE_STORED_MEDIA_FILE_SCORED_OWN_SIZE_LISTING_FACTS":
    "Оценка сохранённого медиафайла рассчитывается по его собственному размеру и данным выдачи; входные данные размера и выдачи применяются только к названию релиза.",
  "error.reason.RULE_TAG_FILTER_ONLY_APPLIES_MANAGED_RULE_SETS":
    "Фильтр тегов применяется только к управляемым наборам правил.",
  "error.reason.RULE_TARGETS_PERSON":
    "Правила запросов не могут быть нацелены на конкретного человека.",
  "error.reason.RULE_TESTING_SAVED_RULE_CANNOT_INCLUDE_DRAFT_EDIT_COPY_CONTEXT":
    "При проверке сохранённого правила нельзя передавать черновик или контекст редактирования либо копирования.",
  "error.reason.RULE_TEST_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_NOT_BOTH":
    "Проверяйте либо название релиза, либо сохранённый медиафайл, но не оба сразу.",
  "error.reason.RULE_TITLE_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_REQUIRED":
    "Необходимо указать тайтл и либо название релиза, либо сохранённый медиафайл.",
  "error.reason.RULE_TITLE_REQUIRED": "Необходимо указать тайтл.",
  "error.reason.RULE_TRACKED_COMMUNITY_PACK":
    "Это правило отслеживается пакетом сообщества. Изменяйте его настройки через отслеживаемый пакет, чтобы изменения состава оставались атомарными, или скопируйте правило для изменения его содержимого.",
  "error.reason.RULE_TRACKED_RULE_PACK_AUTOMATIC_UPDATES_DISABLED":
    "Автоматическое обновление отслеживаемого пакета правил отключено.",
  "error.reason.RULE_TRACKED_RULE_PACK_CHANGED":
    "Отслеживаемый пакет правил изменился; перезагрузите данные и повторите попытку.",
  "error.reason.RULE_TRACKED_RULE_PACK_NOT_FOUND":
    "Отслеживаемый пакет правил не найден.",
  "error.reason.RULE_TRACKED_RULE_REQUIRED":
    "Отсутствует отслеживаемое правило с выбранным значением.",
  "error.reason.RULE_VALIDATION_ERROR": "Ошибка проверки правила.",
  "error.reason.RULE_VALIDATION_FAILED": "Проверка правила не пройдена.",
  "error.reason.RULE_VERSION_INVALID":
    "Версия запрошенного значения не соответствует формату семантического версионирования.",
  "error.reason.RULE_VOTES_CANNOT_NEGATIVE":
    "Количество голосов не может быть отрицательным.",
  "error.reason.SCRIPTS_SCHEDULE_INVALID": "Недопустимое расписание скрипта.",
  "error.reason.SECURITY_ACCOUNT_NO_PASSWORD_SET":
    "Для учётной записи не установлен пароль.",
  "error.reason.SECURITY_ACTOR_CAPABILITY_CLAIM_UNKNOWN":
    "Неизвестное утверждение о возможностях субъекта.",
  "error.reason.SECURITY_ADMIN_DISABLED_SCRYER_DISABLE_DEFAULT_ADMIN":
    "Администратор отключён параметром SCRYER_DISABLE_DEFAULT_ADMIN; удалите этот параметр и перезапустите приложение, прежде чем использовать администратора по умолчанию.",
  "error.reason.SECURITY_ADMIN_PASSWORD_REQUIRED":
    "Необходимо указать пароль администратора.",
  "error.reason.SECURITY_ADMIN_RESET_PASSWORD_REQUIRED":
    "Для SCRYER_ADMIN_PASSWORD_RESET требуется SCRYER_ADMIN_PASSWORD или SCRYER_ADMIN_PASSWORD_FILE.",
  "error.reason.SECURITY_ADMIN_USERNAME_REQUIRED":
    "Необходимо указать имя пользователя администратора.",
  "error.reason.SECURITY_ALTERNATIVE_LOCAL_ADMIN_REQUIRED":
    "Для SCRYER_DISABLE_DEFAULT_ADMIN необходим другой включённый локальный администратор с полными правами и пригодным для входа паролем.",
  "error.reason.SECURITY_ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION":
    "Имя Anonymous зарезервировано для указания субъекта аудита при работе без аутентификации.",
  "error.reason.SECURITY_AUTHENTICATION_SESSION_INVALIDATED":
    "Сеанс аутентификации аннулирован.",
  "error.reason.SECURITY_BACKUP_BUNDLE_ENCRYPTED_REQUIRES_PASSWORD":
    "Этот пакет резервной копии зашифрован и требует пароля.",
  "error.reason.SECURITY_BACKUP_BUNDLE_INVALID":
    "Недопустимый пакет резервной копии.",
  "error.reason.SECURITY_BACKUP_BUNDLE_NOT_FOUND":
    "Пакет резервной копии не найден.",
  "error.reason.SECURITY_BACKUP_BUNDLE_STILL_BEING_CREATED":
    "Пакет резервной копии ещё создаётся.",
  "error.reason.SECURITY_BACKUP_BUNDLE_TABLE_SET_DOES_NOT_MATCH_CURRENT_RESTORE_CATALOG":
    "Набор таблиц пакета резервной копии не соответствует текущему каталогу восстановления: отсутствуют [выбранное значение], лишние [выбранное значение]. Выбранное значение.",
  "error.reason.SECURITY_BACKUP_CHECKSUM_MANIFEST_INCOMPLETE":
    "Манифест контрольных сумм резервной копии неполон: отсутствуют [выбранное значение], лишние [выбранное значение].",
  "error.reason.SECURITY_BACKUP_CHECKSUM_MISMATCH":
    "Контрольная сумма резервной копии для выбранного значения не совпадает.",
  "error.reason.SECURITY_BACKUP_CREATION_FAILED":
    "Не удалось создать резервную копию.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_SUBJECT_UNKNOWN":
    "Неизвестный субъект загрузки резервной копии.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_FILENAME_DOES_NOT_MATCH_REQUEST":
    "Имя файла в токене загрузки резервной копии не совпадает с запросом.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_INVALID":
    "Недопустимый токен загрузки резервной копии.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_KIND_INVALID":
    "Недопустимый вид токена загрузки резервной копии.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR":
    "Субъект токена загрузки резервной копии не совпадает с действующим субъектом.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_SUBJECT_EMPTY":
    "Субъект токена загрузки резервной копии пуст.",
  "error.reason.SECURITY_BACKUP_ENCRYPTION_METADATA_INVALID":
    "Недопустимые метаданные шифрования резервной копии.",
  "error.reason.SECURITY_BACKUP_EXPORT_REQUIRES_CONFIGURED_ENCRYPTION_MASTER_KEY":
    "Для экспорта резервной копии необходим настроенный мастер-ключ шифрования.",
  "error.reason.SECURITY_BACKUP_EXPORT_REQUIRES_NON_EMPTY_PASSWORD":
    "Для экспорта резервной копии необходим непустой пароль.",
  "error.reason.SECURITY_BACKUP_FILENAME_INVALID":
    "Недопустимое имя файла резервной копии.",
  "error.reason.SECURITY_BACKUP_FORMAT_VERSION_UNSUPPORTED":
    "Неподдерживаемая версия формата резервной копии с выбранным значением.",
  "error.reason.SECURITY_BACKUP_MANIFEST_INVALID":
    "Недопустимый манифест резервной копии.",
  "error.reason.SECURITY_BACKUP_MANIFEST_MISSING":
    "Отсутствует манифест резервной копии.",
  "error.reason.SECURITY_BACKUP_METADATA_NOT_FOUND":
    "Метаданные резервной копии не найдены.",
  "error.reason.SECURITY_BACKUP_PART_PATH_ESCAPES_BUNDLE_ROOT":
    "Путь части резервной копии выходит за пределы корневого каталога пакета.",
  "error.reason.SECURITY_BACKUP_PART_PATH_INVALID":
    "Путь части резервной копии должен указывать на обычный файл.",
  "error.reason.SECURITY_BACKUP_PART_PATH_UNREADABLE":
    "Путь части резервной копии недоступен для чтения.",
  "error.reason.SECURITY_BACKUP_PASSWORD_REQUIRED_FULL_BACKUPS":
    "Для полных резервных копий необходимо указать пароль резервной копии.",
  "error.reason.SECURITY_BACKUP_SECRETS_PAYLOAD_INVALID":
    "Недопустимые данные секретов резервной копии.",
  "error.reason.SECURITY_BACKUP_SECRETS_PAYLOAD_MISSING":
    "Отсутствуют данные секретов резервной копии.",
  "error.reason.SECURITY_BOOTSTRAP_ADMINISTRATOR_NO_USABLE_CREDENTIALS":
    "У администратора начальной настройки нет пригодных для входа учётных данных.",
  "error.reason.SECURITY_BOOTSTRAP_ADMIN_PASSWORD_REQUIRED":
    "Для нового администратора начальной настройки требуется SCRYER_ADMIN_PASSWORD или SCRYER_ADMIN_PASSWORD_FILE.",
  "error.reason.SECURITY_CANNOT_CHANGE_LOGIN_STATUS_CURRENT_USER":
    "Нельзя изменить статус разрешения входа для текущего пользователя.",
  "error.reason.SECURITY_CANNOT_CREATE_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE":
    "Нельзя создать admin, пока SCRYER_DISABLE_DEFAULT_ADMIN имеет значение true.",
  "error.reason.SECURITY_CANNOT_DELETE_CURRENT_USER":
    "Нельзя удалить текущего пользователя.",
  "error.reason.SECURITY_CANNOT_DELETE_DEFAULT_ADMIN":
    "Нельзя удалить администратора по умолчанию; вместо этого отключите ему вход.",
  "error.reason.SECURITY_CANNOT_DELETE_LAST_FULL_ADMINISTRATOR":
    "Нельзя удалить последнего администратора с полными правами.",
  "error.reason.SECURITY_CANNOT_DEMOTE_LAST_FULL_ADMINISTRATOR":
    "Нельзя понизить права последнего администратора с полными правами.",
  "error.reason.SECURITY_CANNOT_DISABLE_ADMIN_WITHOUT_ANOTHER_USABLE_ADMINISTRATOR":
    "Нельзя отключить admin, если нет другого администратора, способного войти в систему.",
  "error.reason.SECURITY_CANNOT_DISABLE_LAST_USABLE_FULL_ADMINISTRATOR":
    "Нельзя отключить последнего администратора с полными правами, способного войти в систему.",
  "error.reason.SECURITY_CANNOT_PROVISION_SELECT_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE":
    "Нельзя создать или выбрать admin, пока SCRYER_DISABLE_DEFAULT_ADMIN имеет значение true.",
  "error.reason.SECURITY_CANNOT_REMOVE_OWN_USER_MANAGEMENT_PERMISSION_MANAGEMENT_ACCESS":
    "Нельзя лишить себя доступа к управлению пользователями или разрешениями.",
  "error.reason.SECURITY_CANNOT_RESET_OWN_AUTHENTICATION_FACTORS":
    "Нельзя сбросить собственные факторы аутентификации.",
  "error.reason.SECURITY_CANNOT_RESOLVE_BACKUP_DOWNLOAD_SIGNING_KEY_ACTOR":
    "Не удалось определить ключ подписи загрузки резервной копии для субъекта с выбранным значением.",
  "error.reason.SECURITY_CANNOT_RESOLVE_RELEASE_CANDIDATE_SIGNING_KEY_ACTOR":
    "Не удалось определить ключ подписи кандидата релиза для субъекта с выбранным значением.",
  "error.reason.SECURITY_CONNECTION_DISABLED":
    "Подключение запрошенного значения отключено.",
  "error.reason.SECURITY_CONNECTION_NOT_CONFIGURED":
    "Подключение запрошенного значения не настроено для внешней аутентификации.",
  "error.reason.SECURITY_COULD_NOT_ALLOCATE_SCRYER_USERNAME_EXTERNAL_ACCOUNT":
    "Не удалось выделить имя пользователя Scryer для внешней учётной записи.",
  "error.reason.SECURITY_CREDENTIALS_INVALID": "Недопустимые учётные данные.",
  "error.reason.SECURITY_CREDENTIALS_UNAVAILABLE": "Учётные данные недоступны.",
  "error.reason.SECURITY_CURRENT_PASSWORD_INCORRECT": "Текущий пароль неверен.",
  "error.reason.SECURITY_CURRENT_PASSWORD_INVALID":
    "Недопустимый текущий пароль.",
  "error.reason.SECURITY_CURRENT_PASSWORD_REQUIRED":
    "Необходимо указать текущий пароль.",
  "error.reason.SECURITY_CURRENT_PASSWORD_REQUIRED_REAUTHENTICATION":
    "Для повторной аутентификации необходимо указать текущий пароль.",
  "error.reason.SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION":
    "Учётные данные обнаруживаемого ключа доступа изменились во время аутентификации.",
  "error.reason.SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_RESOLVED_USER":
    "Учётные данные обнаруживаемого ключа доступа не принадлежат определённому пользователю.",
  "error.reason.SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_NOT_FOUND":
    "Учётные данные обнаруживаемого ключа доступа не найдены.",
  "error.reason.SECURITY_EMBY_ACCOUNT_ALREADY_LINKED_INVITED":
    "Учётная запись Emby уже привязана или приглашена.",
  "error.reason.SECURITY_EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY":
    "У подключения Emby нет подтверждённого идентификатора сервера.",
  "error.reason.SECURITY_EMBY_CONNECT_SIGN_DISABLED_CONNECTION":
    "Вход через Emby Connect отключён для этого подключения.",
  "error.reason.SECURITY_EMBY_INVITES_REQUIRE_SAVED_INTEGRATION_API_KEY":
    "Для приглашений Emby необходим сохранённый ключ API интеграции.",
  "error.reason.SECURITY_EMBY_INVITES_REQUIRE_SELECTING_EMBY_USER_PICKER":
    "Для приглашения Emby необходимо выбрать пользователя Emby из списка.",
  "error.reason.SECURITY_EMBY_SIGN_FAILED": "Не удалось войти через Emby.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_HEADER_TRUNCATED":
    "Заголовок зашифрованной резервной копии усечён.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_METADATA_HEADER_TRUNCATED":
    "Заголовок метаданных зашифрованной резервной копии усечён.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_METADATA_TRUNCATED":
    "Метаданные зашифрованной резервной копии усечены.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_PAYLOAD_LENGTH_INVALID":
    "Недопустимая длина данных зашифрованной резервной копии.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_PAYLOAD_LENGTH_TRUNCATED":
    "Поле длины данных зашифрованной резервной копии усечено.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_PAYLOAD_TRUNCATED_INVALID":
    "Данные зашифрованной резервной копии усечены или недопустимы.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_VERSION_UNSUPPORTED":
    "Неподдерживаемая версия зашифрованной резервной копии с выбранным значением.",
  "error.reason.SECURITY_EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD":
    "Пользователи под внешним управлением не могут установить пароль Scryer.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_DISABLED":
    "Внешняя учётная запись отключена.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_DISABLED_INVALID":
    "Внешняя учётная запись отключена; администратор должен восстановить её работоспособность.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_NOT_FOUND":
    "Внешняя учётная запись не найдена.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_NOT_INVITED":
    "Внешняя учётная запись не приглашена.",
  "error.reason.SECURITY_FAILED_COMPUTE_NEXT_BACKUP_DAY":
    "Не удалось вычислить следующий день резервного копирования.",
  "error.reason.SECURITY_FAILED_DECRYPT_BACKUP_BUNDLE":
    "Не удалось расшифровать пакет резервной копии: неверный пароль или повреждённые данные.",
  "error.reason.SECURITY_FAILED_DERIVE_BACKUP_ENCRYPTION_KEY_PASSWORD":
    "Не удалось получить ключ шифрования резервной копии из пароля.",
  "error.reason.SECURITY_FAILED_FINISH_DISCOVERABLE_PASSKEY_AUTHENTICATION":
    "Не удалось завершить аутентификацию с обнаруживаемым ключом доступа.",
  "error.reason.SECURITY_FAILED_FINISH_PASSKEY_AUTHENTICATION":
    "Не удалось завершить аутентификацию с ключом доступа.",
  "error.reason.SECURITY_FAILED_FINISH_PASSKEY_REGISTRATION":
    "Не удалось завершить регистрацию ключа доступа.",
  "error.reason.SECURITY_FAILED_IDENTIFY_DISCOVERABLE_PASSKEY_AUTHENTICATION":
    "Не удалось идентифицировать аутентификацию с обнаруживаемым ключом доступа.",
  "error.reason.SECURITY_FAILED_RESOLVE_CONFIGURED_LOCAL_BACKUP_TIME":
    "Не удалось определить настроенное местное время резервного копирования.",
  "error.reason.SECURITY_FAILED_START_DISCOVERABLE_PASSKEY_AUTHENTICATION":
    "Не удалось начать аутентификацию с обнаруживаемым ключом доступа.",
  "error.reason.SECURITY_FAILED_START_PASSKEY_AUTHENTICATION":
    "Не удалось начать аутентификацию с ключом доступа.",
  "error.reason.SECURITY_FAILED_START_PASSKEY_REGISTRATION":
    "Не удалось начать регистрацию ключа доступа.",
  "error.reason.SECURITY_JELLYFIN_ACCOUNT_ALREADY_LINKED_INVITED":
    "Учётная запись Jellyfin уже привязана или приглашена.",
  "error.reason.SECURITY_JELLYFIN_CONNECTION_AUTHORIZATION_CHANGED":
    "Авторизация подключения Jellyfin изменилась.",
  "error.reason.SECURITY_JELLYFIN_CONNECTION_UNAVAILABLE":
    "Подключение Jellyfin недоступно.",
  "error.reason.SECURITY_JELLYFIN_CREDENTIALS_INVALID":
    "Недопустимые учётные данные Jellyfin.",
  "error.reason.SECURITY_JELLYFIN_INVITES_REQUIRE_SELECTING_JELLYFIN_USER_PICKER":
    "Для приглашения Jellyfin необходимо выбрать пользователя Jellyfin из списка.",
  "error.reason.SECURITY_JELLYFIN_INVITE_API_KEY_REQUIRED":
    "Для приглашений Jellyfin необходим сохранённый ключ API Jellyfin, чтобы Scryer мог определить неизменяемый ID пользователя.",
  "error.reason.SECURITY_JELLYFIN_LINK_AUTHORIZATION_CHANGED":
    "Авторизация привязки Jellyfin изменилась.",
  "error.reason.SECURITY_JELLYFIN_USER_ID_INVALID":
    "Недопустимый ID пользователя Jellyfin.",
  "error.reason.SECURITY_JELLYFIN_USER_VERIFICATION_FAILED":
    "Проверка пользователя Jellyfin не пройдена.",
  "error.reason.SECURITY_LAST_EXTERNAL_LOGIN_REMOVAL_FORBIDDEN":
    "Нельзя отвязать последний внешний способ входа без локального пароля или ключа доступа.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_ALREADY_USED":
    "Проверочный запрос подтверждения входа уже использован.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_EXPIRED":
    "Срок действия проверочного запроса подтверждения входа истёк.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_INVALIDATED":
    "Проверочный запрос подтверждения входа аннулирован.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_UNAVAILABLE":
    "Проверочный запрос подтверждения входа недоступен.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_USER_UNAVAILABLE":
    "Пользователь для подтверждения входа недоступен.",
  "error.reason.SECURITY_MALFORMED_BACKUP_DOWNLOAD_TOKEN":
    "Неверный формат токена загрузки резервной копии.",
  "error.reason.SECURITY_MALFORMED_TOKEN": "Неверный формат токена.",
  "error.reason.SECURITY_MANUAL_BACKUP_ALREADY_RUNNING":
    "Ручное резервное копирование уже выполняется.",
  "error.reason.SECURITY_NEW_PASSWORD_MUST_DIFFER_TEMPORARY_PASSWORD":
    "Новый пароль должен отличаться от временного.",
  "error.reason.SECURITY_NOT_ENABLED":
    "Запрошенное значение не включено для выбранного значения.",
  "error.reason.SECURITY_OAUTH_CLIENT_UNAVAILABLE": "Клиент OAuth недоступен.",
  "error.reason.SECURITY_OAUTH_GRANT_UNAVAILABLE":
    "Разрешение OAuth не подходит для привязки Jellyfin.",
  "error.reason.SECURITY_OAUTH_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES":
    "Токены OAuth не могут содержать возможности субъекта.",
  "error.reason.SECURITY_OAUTH_TOKENS_CANNOT_CARRY_APP_PERMISSIONS":
    "Токены OAuth не могут содержать разрешения приложения.",
  "error.reason.SECURITY_OAUTH_TOKEN_CLAIMS_INVALID":
    "Недопустимые утверждения токена OAuth.",
  "error.reason.SECURITY_PASSKEY_AUTHENTICATION_NOT_CONFIGURED":
    "Аутентификация с ключом доступа не настроена.",
  "error.reason.SECURITY_PASSKEY_AUTHENTICATION_RESPONSE_PAYLOAD_INVALID":
    "Недопустимые данные ответа аутентификации с ключом доступа.",
  "error.reason.SECURITY_PASSKEY_AUTHENTICATION_UNAVAILABLE_WHILE_FORM_LOGIN_DISABLED":
    "Аутентификация с ключом доступа недоступна, пока вход через форму отключён.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_DOES_NOT_BELONG_CURRENT_USER":
    "Проверочный запрос ключа доступа не принадлежит текущему пользователю.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_DOES_NOT_BELONG_LOGIN_VERIFICATION":
    "Проверочный запрос ключа доступа не относится к этому подтверждению входа.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_EXPIRED":
    "Срок действия проверочного запроса ключа доступа истёк.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_ACTOR_BOUND":
    "Проверочный запрос ключа доступа не привязан к субъекту.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_AUTHENTICATION_CEREMONY":
    "Проверочный запрос ключа доступа не относится к процедуре аутентификации.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_FOUND":
    "Проверочный запрос ключа доступа не найден.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_REGISTRATION_CEREMONY":
    "Проверочный запрос ключа доступа не относится к процедуре регистрации.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_SESSION_INVALIDATED":
    "Сеанс проверочного запроса ключа доступа аннулирован.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_STORED_AUTHENTICATION_CEREMONY":
    "В проверочном запросе ключа доступа сохранена процедура аутентификации.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_STORED_REGISTRATION_CEREMONY":
    "В проверочном запросе ключа доступа сохранена процедура регистрации.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_UNEXPECTED_AUTHENTICATION_PURPOSE":
    "У проверочного запроса ключа доступа неожиданная цель аутентификации.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_UNEXPECTED_REGISTRATION_PURPOSE":
    "У проверочного запроса ключа доступа неожиданная цель регистрации.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION":
    "Учётные данные ключа доступа изменились во время аутентификации.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_CHALLENGE_USER":
    "Учётные данные ключа доступа не принадлежат пользователю проверочного запроса.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_LOGIN_VERIFICATION":
    "Учётные данные ключа доступа не относятся к этому подтверждению входа.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_ID_ALREADY_REGISTERED":
    "Ключ доступа с этим ID учётных данных уже зарегистрирован.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_NOT_FOUND":
    "Учётные данные ключа доступа не найдены.",
  "error.reason.SECURITY_PASSKEY_FACTOR_UNAVAILABLE":
    "Фактор аутентификации с ключом доступа больше недоступен.",
  "error.reason.SECURITY_PASSKEY_REAUTHENTICATION_CHALLENGE_BELONGS_ANOTHER_USER":
    "Проверочный запрос повторной аутентификации с ключом доступа принадлежит другому пользователю.",
  "error.reason.SECURITY_PASSKEY_REAUTHENTICATION_UNAVAILABLE_ACCOUNT":
    "Повторная аутентификация с ключом доступа недоступна для этой учётной записи.",
  "error.reason.SECURITY_PASSKEY_REGISTRATION_RESPONSE_PAYLOAD_INVALID":
    "Недопустимые данные ответа регистрации ключа доступа.",
  "error.reason.SECURITY_PASSWORD_CHANGE_ENROLLMENT_TOKEN_CLAIMS_INVALID":
    "Недопустимые утверждения токена регистрации для смены пароля.",
  "error.reason.SECURITY_PASSWORD_HASH_VERSION_UNSUPPORTED":
    "Неподдерживаемая версия хеша пароля.",
  "error.reason.SECURITY_PASSWORD_OUT_OF_RANGE":
    "Пароль должен содержать не менее {{minLength}} символов.",
  "error.passwordLengthRequirement":
    "Пароль не соответствует настроенным требованиям к длине.",
  "error.reason.SECURITY_PASSWORD_REAUTHENTICATION_UNAVAILABLE_ACCOUNT":
    "Повторная аутентификация по паролю недоступна для этой учётной записи.",
  "error.reason.SECURITY_PASSWORD_REQUIRED": "Необходимо указать пароль.",
  "error.reason.SECURITY_PLEX_ACCOUNT_ALREADY_LINKED_INVITED":
    "Учётная запись Plex уже привязана или приглашена.",
  "error.reason.SECURITY_PLEX_INVITES_REQUIRE_SELECTING_PLEX_USER_PICKER":
    "Для приглашения в Plex выберите пользователя Plex из списка.",
  "error.reason.SECURITY_PLEX_INVITE_TOKEN_REQUIRED":
    "Для приглашений в Plex нужен сохранённый токен Plex, чтобы Scryer мог определить неизменяемый идентификатор пользователя.",
  "error.reason.SECURITY_PLEX_SERVER_DISCOVERY_REQUIRED_BEFORE_USING_PLEX_AUTH":
    "Перед использованием Plex для аутентификации необходимо обнаружить сервер Plex.",
  "error.reason.SECURITY_PREPARED_BACKUP_DIRECTORY_CONTAINS_UNSUPPORTED_ENTRY":
    "Подготовленный каталог резервной копии содержит неподдерживаемый элемент с выбранным значением.",
  "error.reason.SECURITY_RECOVERY_ADMIN_LOGIN_MANAGED_ENVIRONMENT":
    "Вход администратора восстановления управляется окружением.",
  "error.reason.SECURITY_RECOVERY_ADMIN_USER_EXTERNALLY_MANAGED_CANNOT_REPAIRED_LOCAL_LOGIN":
    "Учётная запись администратора восстановления управляется извне; восстановить для неё локальный вход невозможно.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_EXPIRED":
    "Срок действия кандидата на загрузку релиза истёк; повторите поиск.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_EMPTY_EPISODE_SET_SCOPE":
    "Область действия токена кандидата на загрузку релиза содержит пустой набор эпизодов.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_KIND_INVALID":
    "Недопустимый тип токена кандидата на загрузку релиза.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_COLLECTION_SCOPE_ID":
    "В токене кандидата на загрузку релиза отсутствует идентификатор коллекции для области действия.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SCOPE_ID":
    "В токене кандидата на загрузку релиза отсутствует идентификатор эпизода для области действия.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SET_SCOPE_ID":
    "В токене кандидата на загрузку релиза отсутствует идентификатор набора эпизодов для области действия.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_SERIES_MOVIE_SCOPE_ID":
    "В токене кандидата на загрузку релиза отсутствует идентификатор фильма сериала для области действия.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_HINT":
    "Для токена кандидата на загрузку релиза требуется указание источника.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_TITLE":
    "Для токена кандидата на загрузку релиза требуется название источника.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_SCOPE_DOES_NOT_MATCH_REQUEST":
    "Область действия токена кандидата на загрузку релиза не соответствует запросу.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR":
    "Субъект токена кандидата на загрузку релиза не соответствует исполнителю.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_TITLE_DOES_NOT_MATCH_REQUEST":
    "Тайтл в токене кандидата на загрузку релиза не соответствует запросу.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_UNKNOWN_SCOPE":
    "Неизвестная область действия токена кандидата на загрузку релиза.",
  "error.reason.SECURITY_RELEASE_REPORTS_SEEDERS_BELOW_MINIMUM_INDEXER":
    "В релизе указано выбранное число сидов, что меньше выбранного минимального значения для этого индексатора.",
  "error.reason.SECURITY_REQUESTED_LOGIN_VERIFICATION_FACTOR_UNAVAILABLE":
    "Запрошенный фактор проверки входа недоступен.",
  "error.reason.SECURITY_RESERVED_INSTANCE_RECOVERY":
    "Запрошенное значение зарезервировано для восстановления экземпляра.",
  "error.reason.SECURITY_RESTRICTED_AUTHENTICATION_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES":
    "Токены аутентификации с ограничениями не могут содержать полномочия исполнителя.",
  "error.reason.SECURITY_SCRYER_ADMIN_USERNAME_CONFLICTS_EXISTING_USERNAME":
    "SCRYER_ADMIN_USERNAME конфликтует с существующим именем пользователя; используйте точное написание сохранённого имени.",
  "error.reason.SECURITY_SCRYER_ADMIN_USERNAME_MUST_NAME_NON_RESERVED_LOCAL_ADMINISTRATOR":
    "SCRYER_ADMIN_USERNAME должен указывать на локального администратора, имя которого не зарезервировано.",
  "error.reason.SECURITY_SELECTED_BOOTSTRAP_ACCOUNT_MUST_ALREADY_ENABLED_LOCAL_FULL_ADMINISTRATOR":
    "Выбранная учётная запись начальной настройки уже должна быть включённым локальным администратором с полными правами; при потере доступа используйте восстановление.",
  "error.reason.SECURITY_SELECTED_EMBY_USER_NOT_FOUND_CONFIGURED_SERVER":
    "Выбранный пользователь Emby не найден на настроенном сервере.",
  "error.reason.SECURITY_SELECTED_JELLYFIN_USER_NOT_FOUND_CONFIGURED_SERVER":
    "Выбранный пользователь Jellyfin не найден на настроенном сервере.",
  "error.reason.SECURITY_SELECTED_PLEX_USER_NOT_FOUND_CONFIGURED_ACCOUNT":
    "Выбранный пользователь Plex не найден в настроенной учётной записи.",
  "error.reason.SECURITY_SESSION_DURATION_OUT_OF_RANGE":
    "Продолжительность сеанса должна составлять от 1 до 365 дней.",
  "error.reason.SECURITY_TOKEN_INVALID": "Недопустимый токен.",
  "error.reason.SECURITY_TOKEN_SUBJECT_NO_LONGER_EXISTS":
    "Субъект токена больше не существует.",
  "error.reason.SECURITY_TOKEN_SUBJECT_UNKNOWN": "Неизвестный субъект токена.",
  "error.reason.SECURITY_TOTP_ALREADY_ENABLED": "TOTP уже включён.",
  "error.reason.SECURITY_TOTP_CHALLENGE_NOT_FOUND":
    "Запрос проверки TOTP не найден.",
  "error.reason.SECURITY_TOTP_ENROLLMENT_CHALLENGE_EXPIRED":
    "Срок действия запроса настройки TOTP истёк.",
  "error.reason.SECURITY_USERNAME_REQUIRED":
    "Необходимо указать имя пользователя.",
  "error.reason.SECURITY_USER_ALREADY_EXISTS":
    "Пользователь с выбранным значением уже существует.",
  "error.reason.SECURITY_USER_NOT_FOUND": "Пользователь не найден.",
  "error.reason.SECURITY_USE_CHANGE_OWN_PASSWORD_UPDATE_OWN_PASSWORD":
    "Чтобы изменить собственный пароль, используйте change_own_password.",
  "error.reason.SECURITY_V2_PASSWORD_HASH_INVALID":
    "Недопустимый хеш пароля v2.",
  "error.reason.SECURITY_VERIFIED_EXTERNAL_IDENTITY_CONNECTION_DID_NOT_MATCH_REQUESTED_CONNECTION":
    "Подключение проверенной внешней учётной записи не соответствует запрошенному подключению.",
  "error.reason.SECURITY_VERIFIED_EXTERNAL_IDENTITY_PROVIDER_DID_NOT_MATCH_REQUESTED_PROVIDER":
    "Провайдер проверенной внешней учётной записи не соответствует запрошенному провайдеру.",
  "error.reason.SERVICES_ACTION_PERMISSION_DENIED":
    "У вас нет разрешения на это действие.",
  "error.reason.SERVICES_COMPLETED_DOWNLOAD_NOT_FOUND":
    "Завершённая загрузка не найдена.",
  "error.reason.SERVICES_DOWNLOAD_CLIENT_ASSIGNED_PROXY_WHICH_DISABLED":
    "Клиенту загрузки с выбранным значением назначен прокси с выбранным значением, который отключён.",
  "error.reason.SERVICES_DOWNLOAD_CLIENT_ITEM_ID_REQUIRED":
    "Необходим идентификатор элемента клиента загрузки.",
  "error.reason.SERVICES_DOWNLOAD_CLIENT_REFERENCES_PROXY_CONFIGURATION_WHICH_NOT_FOUND":
    "Клиент загрузки ссылается на конфигурацию прокси, которая не найдена.",
  "error.reason.SERVICES_LIBRARY_ACCESS_DENIED":
    "У вас нет доступа к этой библиотеке.",
  "error.reason.SERVICES_MAINTENANCE_OPERATION_PROGRESS":
    "Выполняется операция обслуживания.",
  "error.reason.SERVICES_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND":
    "В ответе с метаданными фильма не найдено отсутствующее название.",
  "error.reason.SERVICES_TITLE_IDENTITY_REQUIRED":
    "Необходим идентификатор тайтла.",
  "error.reason.SERVICES_TITLE_IDS_OUT_OF_RANGE":
    "Идентификаторы тайтлов должны быть положительными.",
  "error.reason.SETTINGS_ACQUISITION_POLL_INTERVAL_OUT_OF_RANGE":
    "Интервал опроса для получения контента должен составлять не менее 1 секунды.",
  "error.reason.SETTINGS_ACQUISITION_THRESHOLDS_CANNOT_NEGATIVE":
    "Пороговые значения получения контента не могут быть отрицательными.",
  "error.reason.SETTINGS_ACQUISITION_WALK_INTERVAL_OUT_OF_RANGE":
    "Интервал обхода для получения контента должен составлять не менее 1 секунды.",
  "error.reason.SETTINGS_ANIME_SPECIFIC_SETTINGS_REQUIRE_ANIME_LIBRARY":
    "Настройки аниме требуют библиотеки аниме.",
  "error.reason.SETTINGS_ANIME_SPECIFIC_SETTINGS_REQUIRE_SCOPE_ANIME":
    "Настройки аниме требуют области действия anime.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUPS_REQUIRE_BACKUP_KEY":
    "Для автоматического резервного копирования необходим ключ резервной копии.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUP_KEY_CANNOT_CLEARED_WHILE_AUTOMATIC_BACKUPS_DISABLED":
    "Нельзя удалить ключ автоматического резервного копирования, пока автоматическое резервное копирование включено.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUP_KEY_CANNOT_REPLACED_CLEARED_SAME_REQUEST":
    "Нельзя одновременно заменить и удалить ключ автоматического резервного копирования в одном запросе.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUP_KEY_OUT_OF_RANGE":
    "Ключ автоматического резервного копирования должен содержать не менее выбранного числа непробельных символов.",
  "error.reason.SETTINGS_BACKUP_PATH_INVALID":
    "Путь резервного копирования должен указывать на каталог.",
  "error.reason.SETTINGS_BACKUP_PATH_NOT_WRITABLE":
    "Путь резервного копирования недоступен для записи.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_GLOBAL_DEFAULT":
    "Нельзя удалить выбранный профиль качества: он используется по умолчанию глобально.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_ALLOWS_REQUESTS":
    "Нельзя удалить выбранный профиль качества: выбранная библиотека разрешает его для запросов.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_REFERENCES":
    "Нельзя удалить выбранный профиль качества: на него ссылается выбранная библиотека.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_NOT_CONFIGURED":
    "Нельзя удалить выбранный профиль качества: он настроен для выбранного значения.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_PENDING_MEDIA_REQUEST_S_REFERENCE":
    "Нельзя удалить выбранный профиль качества: на него ссылается выбранное число ожидающих запросов на медиаконтент.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_TITLE_S_REFERENCE":
    "Нельзя удалить выбранный профиль качества: на него ссылается выбранное число тайтлов.",
  "error.reason.SETTINGS_CANNOT_SAVE_RESET_TRUSTED_PROXIES_TOGETHER":
    "Нельзя одновременно сохранить и сбросить доверенные прокси.",
  "error.reason.SETTINGS_CATALOG_VIEW_COLUMNS_INVALID":
    "Столбцы представления каталога должны соответствовать сохранённому классу устройства и типу медиаконтента.",
  "error.reason.SETTINGS_CERTIFICATE_BUNDLE_EMPTY":
    "Набор доверенных сертификатов не содержит читаемого сертификата X.509.",
  "error.reason.SETTINGS_CHANGE_DEFAULT_ADMIN_PASSWORD_BEFORE_ENABLING_FORM_LOGIN":
    "Измените пароль администратора по умолчанию, прежде чем включать вход через форму.",
  "error.reason.SETTINGS_CONFIGURE_ENABLED_FULL_ADMINISTRATOR_LOGIN_BEFORE_ENABLING_FORM_LOGIN":
    "Настройте включённую учётную запись администратора с полными правами, прежде чем включать вход через форму.",
  "error.reason.SETTINGS_CONVERGENCE_PER_CYCLE_SCOPE_CEILING_OUT_OF_RANGE":
    "Максимальное число объектов согласования за цикл должно быть не меньше 1.",
  "error.reason.SETTINGS_DEFAULT_LIBRARY_NOT_FOUND":
    "Библиотека по умолчанию не найдена.",
  "error.reason.SETTINGS_DEFAULT_MOVIE_AVAILABILITY_INVALID":
    "Доступность фильмов по умолчанию должна иметь значение announced, in_cinemas или released.",
  "error.reason.SETTINGS_DELAY_PROFILE_ID_REQUIRED":
    "Необходим идентификатор профиля задержки.",
  "error.reason.SETTINGS_DELAY_PROFILE_NAMES_UNKNOWN_LIBRARY":
    "Выбранный профиль задержки указывает на неизвестную выбранную библиотеку.",
  "error.reason.SETTINGS_DELAY_PROFILE_NOT_FOUND":
    "Профиль задержки не найден.",
  "error.reason.SETTINGS_DOWNLOAD_CLIENT_ROUTING_ENTRY_REQUIRES_CLIENT_ID":
    "Для записи маршрутизации клиента загрузки необходим client_id.",
  "error.reason.SETTINGS_EACH_TRUSTED_CERTIFICATE_ENTRY_INVALID":
    "Каждая запись доверенного сертификата должна содержать ровно один сертификат X.509.",
  "error.reason.SETTINGS_FAILED_CREATE_BACKUP_DIRECTORY":
    "Не удалось создать каталог резервных копий.",
  "error.reason.SETTINGS_FAILED_PARSE_TRAILING_PEM_CONTENT_TRUSTED_CERTIFICATE":
    "Не удалось разобрать завершающие данные PEM доверенного сертификата.",
  "error.reason.SETTINGS_FAILED_PARSE_TRUSTED_CERTIFICATE_PEM_BLOCK":
    "Не удалось разобрать блок PEM доверенного сертификата.",
  "error.reason.SETTINGS_FAILED_REMOVE_BACKUP_PATH_PROBE":
    "Не удалось удалить проверочный файл пути резервного копирования.",
  "error.reason.SETTINGS_FORM_LOGIN_REQUIRED_DEFAULT_ADMIN_DISABLED":
    "Нельзя отключить вход через форму, пока SCRYER_DISABLE_DEFAULT_ADMIN имеет значение true: при работе без входа используется учётная запись администратора по умолчанию. Сначала удалите эту настройку и перезапустите приложение.",
  "error.reason.SETTINGS_HISTORY_RETENTION_DAYS_OUT_OF_RANGE":
    "Срок хранения истории должен составлять не менее 1 дня.",
  "error.reason.SETTINGS_IMAGE_CACHE_MAXIMUM_SIZE_OUT_OF_RANGE":
    "Максимальный размер кеша изображений должен составлять не менее 1 MiB.",
  "error.reason.SETTINGS_INDEXER_ROUTING_ENTRY_REQUIRES_INDEXER_ID":
    "Для записи маршрутизации индексатора необходим indexer_id.",
  "error.reason.SETTINGS_LANGUAGE_OUT_OF_RANGE":
    "Язык должен быть задан кодом длиной не более выбранного числа букв, цифр или дефисов.",
  "error.reason.SETTINGS_LIBRARY_ACCESS_DENIED":
    "У вас нет доступа к этой библиотеке.",
  "error.reason.SETTINGS_LIBRARY_NOT_FOUND": "Библиотека не найдена.",
  "error.reason.SETTINGS_LIMIT_REQUIRED_OUT_OF_RANGE":
    "Необходимо указать лимит больше нуля.",
  "error.reason.SETTINGS_MEDIA_SETTING_CHANGE_REQUIRED":
    "Необходимо изменить хотя бы одну настройку медиаконтента.",
  "error.reason.SETTINGS_METADATA_LANGUAGE_INVALID":
    "Язык метаданных должен иметь одно из значений: eng, spa, fra, deu, ita, por, kor, zho или jpn.",
  "error.reason.SETTINGS_MONITOR_SPECIALS_ONLY_VALID_ANIME_LIBRARIES":
    "Monitor_specials допустим только для библиотек аниме.",
  "error.reason.SETTINGS_MOVIES_TABLE_SETTINGS_CANNOT_INCLUDE_EPISODES_COLUMN":
    "Настройки таблицы фильмов не могут включать столбец эпизодов.",
  "error.reason.SETTINGS_MOVIE_AVAILABILITY_SETTINGS_REQUIRE_MOVIE_LIBRARY":
    "Настройки доступности фильмов требуют библиотеки фильмов.",
  "error.reason.SETTINGS_MOVIE_AVAILABILITY_SETTINGS_REQUIRE_SCOPE_MOVIE":
    "Настройки доступности фильмов требуют области действия movie.",
  "error.reason.SETTINGS_MOVIE_RELEASE_MARKET_INVALID":
    "Рынок выпуска фильма должен быть задан двухбуквенным кодом страны ISO-3166.",
  "error.reason.SETTINGS_MUST_ABSOLUTE_PATH_COMMAND_NAME_WITHOUT_PATH_SEPARATORS":
    "Запрошенное значение должно быть абсолютным путём или именем команды без разделителей пути.",
  "error.reason.SETTINGS_MUST_RRGGBB_HEX_COLOR":
    "Запрошенное значение должно быть шестнадцатеричным цветом в формате #RRGGBB.",
  "error.reason.SETTINGS_PASSWORD_MINIMUM_LENGTH_OUT_OF_RANGE":
    "Минимальная длина пароля должна быть не меньше выбранного значения.",
  "error.reason.SETTINGS_PERMISSIONS_CHOWN_GROUP_SETTING_VALUE_INVALID":
    "Недопустимое выбранное значение настройки permissions.chown_group: числовой gid выходит за допустимый диапазон.",
  "error.reason.SETTINGS_PERSONA_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE":
    "Необходимо указать Persona, если inherit_global имеет значение false.",
  "error.reason.SETTINGS_PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME":
    "Plexmatch_write_on_import допустим только для сериалов и аниме.",
  "error.reason.SETTINGS_PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME_LIBRARIES":
    "Plexmatch_write_on_import допустим только для библиотек сериалов и аниме.",
  "error.reason.SETTINGS_PROFILE_ID_REQUIRED": "Необходимо указать Profile_id.",
  "error.reason.SETTINGS_PROFILE_ID_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE":
    "Необходимо указать Profile_id, если inherit_global имеет значение false.",
  "error.reason.SETTINGS_QUALITY_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE":
    "Выбранные идентификаторы профилей качества отличаются только регистром ASCII.",
  "error.reason.SETTINGS_QUALITY_PROFILE_ID_AMBIGUOUS_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE":
    "Выбранный идентификатор профиля качества неоднозначен: идентификаторы профилей отличаются только регистром ASCII.",
  "error.reason.SETTINGS_QUALITY_PROFILE_ID_REQUIRED":
    "Необходим идентификатор профиля качества.",
  "error.reason.SETTINGS_QUALITY_PROFILE_NOT_FOUND":
    "Профиль качества не найден.",
  "error.reason.SETTINGS_QUALITY_PROFILE_UNKNOWN":
    "Неизвестный выбранный профиль качества.",
  "error.reason.SETTINGS_RECYCLE_BIN_PATH_UNSAFE":
    "Путь корзины небезопасен или пересекается с управляемыми медиафайлами.",
  "error.reason.SETTINGS_RECYCLE_BIN_RETENTION_OUT_OF_RANGE":
    "Срок хранения в корзине должен составлять от 1 до выбранного числа дней.",
  "error.reason.SETTINGS_REQUEST_QUALITY_PROFILE_UNKNOWN":
    "Неизвестный профиль качества запросов с выбранным значением.",
  "error.reason.SETTINGS_RE_CONVERGE_BACKSTOP_CANNOT_NEGATIVE":
    "Предельный интервал повторного согласования не может быть отрицательным.",
  "error.reason.SETTINGS_ROOT_FOLDER_PATH_REQUIRED":
    "Необходимо указать путь корневой папки.",
  "error.reason.SETTINGS_ROOT_REQUIRED":
    "Необходима хотя бы одна корневая папка.",
  "error.reason.SETTINGS_SEASON_FOLDERS_ONLY_VALID_SERIES_ANIME_LIBRARIES":
    "Папки сезонов допустимы только для библиотек сериалов и аниме.",
  "error.reason.SETTINGS_SEASON_FOLDER_TEMPLATES_ONLY_SUPPORTED_SERIES_ANIME":
    "Шаблоны папок сезонов поддерживаются только для сериалов и аниме.",
  "error.reason.SETTINGS_SESSION_DURATION_OUT_OF_RANGE":
    "Продолжительность сеанса должна составлять от 1 до 365 дней.",
  "error.reason.SETTINGS_SETTING_VALUE_INVALID":
    "Недопустимое выбранное значение выбранной настройки: ожидается восьмеричная маска из 3 или 4 цифр.",
  "error.reason.SETTINGS_SOURCE_REQUIRED": "Необходимо указать источник.",
  "error.reason.SETTINGS_SUBTITLE_MINIMUM_SCORE_PERCENTAGES_OUT_OF_RANGE":
    "Минимальная оценка субтитров в процентах должна быть от 0 до 100.",
  "error.reason.SETTINGS_SUBTITLE_SEARCH_INTERVAL_OUT_OF_RANGE":
    "Интервал поиска субтитров должен составлять не менее 1 часа.",
  "error.reason.SETTINGS_SUBTITLE_SYNC_MAX_OFFSET_CANNOT_NEGATIVE":
    "Максимальное смещение синхронизации субтитров не может быть отрицательным.",
  "error.reason.SETTINGS_SUBTITLE_SYNC_THRESHOLD_PERCENTAGES_OUT_OF_RANGE":
    "Пороговые значения синхронизации субтитров в процентах должны быть от 0 до 100.",
  "error.reason.SETTINGS_TABLE_COLUMN_ORDER_OUT_OF_RANGE":
    "Порядковый номер столбца таблицы должен быть не меньше 0.",
  "error.reason.SETTINGS_TABLE_COLUMN_UNSUPPORTED":
    "Неподдерживаемый выбранный столбец выбранной таблицы.",
  "error.reason.SETTINGS_TITLE_IMAGE_CACHE_REFRESH_ALREADY_RUNNING":
    "Обновление кеша изображений тайтлов уже выполняется.",
  "error.reason.SETTINGS_TRUSTED_CERTIFICATE_BUNDLE_INVALID":
    "Набор доверенных сертификатов должен содержать сертификаты X.509 в кодировке PEM.",
  "error.reason.SETTINGS_TRUSTED_CERTIFICATE_BUNDLE_MAY_ONLY_CONTAIN_X_509_CERTIFICATE_PEM_BLOCKS":
    "Набор доверенных сертификатов может содержать только блоки PEM сертификатов X.509.",
  "error.reason.SETTINGS_UI_TABLE_COLUMN_SETTING_COLUMN_DUPLICATED":
    "Повторяющаяся настройка столбца таблицы интерфейса для выбранных значений и выбранного столбца.",
  "error.reason.SETTINGS_VIEW_QUALITY_PROFILES_PERMISSION_DENIED":
    "У вас нет разрешения на просмотр профилей качества.",
  "error.reason.SETTINGS_VIEW_RECYCLE_BIN_SETTINGS_PERMISSION_DENIED":
    "У вас нет разрешения на просмотр настроек корзины.",
  "error.reason.SETTINGS_VIEW_VERIFICATION_SETTINGS_PERMISSION_DENIED":
    "У вас нет разрешения на просмотр настроек проверки.",
  "error.reason.SUBTITLES_ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_SUBTITLE_ARCHIVE_FORMAT":
    "Плагин архивов не поддерживает этот формат архива субтитров.",
  "error.reason.SUBTITLES_ARCHIVE_PLUGIN_OUTPUT_ESCAPED_OUTPUT_ROOT":
    "Результат работы плагина архивов вышел за пределы корневого каталога вывода.",
  "error.reason.SUBTITLES_ARCHIVE_SUBTITLE_NOT_FOUND":
    "Архив субтитров не содержит файла субтитров поддерживаемого формата.",
  "error.reason.SUBTITLES_FILE_TOO_SMALL_SUBTITLE_FILE_HASH":
    "Файл слишком мал для вычисления хеша файла субтитров.",
  "error.reason.SUBTITLES_MEDIA_FILE_NOT_FOUND": "Медиафайл не найден.",
  "error.reason.SUBTITLES_NO_ENABLED_SUBTITLE_PROVIDERS_NOT_CONFIGURED":
    "Не настроено ни одного включённого провайдера субтитров.",
  "error.reason.SUBTITLES_SIDECAR_SYMLINK_FORBIDDEN":
    "Сопутствующий файл субтитров не должен быть символической ссылкой.",
  "error.reason.SUBTITLES_SUBTITLE_ARCHIVE_CONTAINS_TOO_MANY_FILES":
    "Архив субтитров содержит слишком много файлов.",
  "error.reason.SUBTITLES_SUBTITLE_ARCHIVE_MULTIPLE_EQUALLY_RANKED_SUBTITLE_FILES":
    "В архиве субтитров несколько файлов с одинаковым приоритетом.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACTS_CONTAIN_TOO_MANY_FILES":
    "Материалы субтитров содержат слишком много файлов.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACT_EXCEEDS_NESTING_INPUT_LIMITS":
    "Материал субтитров превышает ограничения вложенности или входных данных.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACT_EXPANDS_BEYOND_BYTE_LIMIT":
    "После распаковки материал субтитров превышает выбранный лимит в байтах.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACT_FORMAT_UNSUPPORTED":
    "Неподдерживаемый формат материала субтитров.",
  "error.reason.SUBTITLES_SUBTITLE_FORMAT_UNSUPPORTED":
    "Неподдерживаемый формат субтитров.",
  "error.reason.SUBTITLES_SUBTITLE_LANGUAGE_CODE_INVALID":
    "Недопустимый код языка субтитров.",
  "error.reason.SUBTITLES_SUBTITLE_PLUGIN_PROVIDER_TYPE_NOT_FOUND":
    "Плагин субтитров для данного типа провайдера не найден.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_FIELD_REQUIRED":
    "Необходимо указать хотя бы одно поле конфигурации провайдера субтитров.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_JSON_INVALID":
    "config_json провайдера субтитров должен быть объектом JSON.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_NAME_REQUIRED":
    "Имя конфигурации провайдера субтитров не должно быть пустым.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND":
    "Конфигурация провайдера субтитров не найдена.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_NOT_NOT_FOUND":
    "Провайдер субтитров не настроен и не найден.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_TYPE_REQUIRED":
    "Тип провайдера субтитров не должен быть пустым.",
  "error.reason.SUBTITLES_SUBTITLE_SIDECAR_CHANGED_DURING_CONTENT_PROBE":
    "Сопутствующий файл субтитров изменился во время проверки содержимого.",
  "error.reason.SUBTITLES_SUBTITLE_SIDECAR_EXCEEDS_CONTENT_PROBE_SIZE_LIMIT":
    "Сопутствующий файл субтитров превышает ограничение размера для проверки содержимого.",
  "error.reason.SUBTITLES_SUBTITLE_SIDECAR_INVALID":
    "Сопутствующий файл субтитров должен быть обычным файлом.",
  "error.reason.SUBTITLES_TITLE_NOT_FOUND": "Тайтл не найден.",
  "error.reason.TITLE_NOT_FOUND": "Не удалось найти тайтл.",
  "error.reason.WORKFLOW_ACQUISITION_RECEIPT_REQUIRED":
    "Для поиска в рамках обслуживания необходимо принятое подтверждение запуска соответствующего задания получения контента.",
  "error.reason.WORKFLOW_CLEANUP_IDENTITY_REQUIRED":
    "Для ручной очистки необходимо указать тайтл и тип медиаконтента.",
  "error.reason.WORKFLOW_DOWNLOAD_AWAITING_RETRY_RECONCILIATION":
    "Загрузка ожидает согласования повторной попытки.",
  "error.reason.WORKFLOW_DOWNLOAD_BINDING_CHANGED_DURING_RETRY":
    "Привязка загрузки изменилась во время повторной попытки; необходимо согласование.",
  "error.reason.WORKFLOW_DOWNLOAD_IDENTITY_UNRESOLVED":
    "У этой загрузки ещё нет устойчивого идентификатора; её исходные файлы остаются удерживаемыми.",
  "error.reason.WORKFLOW_DOWNLOAD_RETRY_OWNERSHIP_LOST":
    "Повторная попытка загрузки больше не владеет этим заданием.",
  "error.reason.WORKFLOW_DOWNLOAD_SUBMISSION_NOT_FOUND":
    "Не удалось найти отправленную загрузку.",
  "error.reason.WORKFLOW_IMPORT_NOT_FOUND": "Не удалось найти импорт.",
  "error.reason.WORKFLOW_IMPORT_RELEASE_STALE":
    "Импорт изменился во время запроса на его освобождение; обновите очередь.",
  "error.reason.WORKFLOW_IMPORT_RETRY_DOWNLOAD_NOT_FOUND":
    "Не удалось найти загрузку повторной попытки импорта.",
  "error.reason.WORKFLOW_IMPORT_RUNNING":
    "Эта загрузка ещё импортируется; освободите её исходные файлы после завершения импорта.",
  "error.reason.WORKFLOW_IMPORT_SOURCES_HELD":
    "Другой импорт этой загрузки удерживает её исходные файлы; обновите очередь.",
  "error.reason.WORKFLOW_OPERATION_NOT_FOUND": "Не удалось найти операцию.",
  "error.reason.WORKFLOW_PASSWORD_RETRY_AWAITING_RECONCILIATION":
    "Повторная попытка с паролем ожидает согласования; привязка сохранена.",
  "error.reason.WORKFLOW_RETRY_DOWNLOAD_MISMATCH":
    "Ответ повторной попытки относится к другой загрузке; необходимо согласование.",
  "error.reason.WORKFLOW_RETRY_PROGRESS_CONFLICT":
    "Ответ повторной попытки противоречит наблюдаемому ходу выполнения; необходимо согласование.",
  "error.reason.WORKFLOW_SOURCE_CLEANUP_ALREADY_CLAIMED":
    "Очистка исходных файлов уже владеет этой загрузкой.",
  "error.ruleUnknownInputPath":
    "Неизвестный путь входных данных правила: {{path}}. Выберите документированное поле.",
  "error.ruleDynamicInputPath":
    "Этот путь входных данных правила нельзя определить статически: {{path}}. Укажите поле напрямую.",
  "error.rulePackageRequired":
    "Правило должно объявлять назначенный ему пакет.",
  "error.ruleInputImportUnsupported":
    "Обращайтесь к входным данным правила напрямую, а не импортируйте их под другим именем.",
  "error.ruleMatchRequired":
    "Определите логическое правило match для этого условия обслуживания.",
  "error.ruleVoteRequired":
    "Определите approve, deny, manual или tags для этого условия запросов.",
  "error.reason.INTERFACE_ACTION_DEFINITION_CONFLICT":
    "Выберите действие или последовательность действий, но не оба варианта.",
  "error.reason.INTERFACE_ACTION_DEFINITION_REQUIRED":
    "Выберите действие или последовательность действий.",
  "error.reason.INTERFACE_ACTION_SEQUENCE_INPUT_REQUIRED":
    "Для создания последовательности используйте входные данные последовательности действий.",
  "error.reason.INTERFACE_ACTION_SCHEMA_VERSION_INVALID":
    "Версия схемы последовательности действий не может быть отрицательной.",
  "error.reason.INTERFACE_ACTION_UNMONITOR_PARAMETERS_INVALID":
    "Шаг отключения отслеживания принимает только параметр включения дочерних объектов.",
  "error.reason.INTERFACE_ACTION_UNMONITOR_DESCENDANTS_REQUIRED":
    "Укажите, распространяется ли шаг отключения отслеживания на дочерние объекты.",
  "error.reason.INTERFACE_ACTION_QUALITY_PARAMETERS_INVALID":
    "Шаг изменения профиля качества принимает только целевой профиль.",
  "error.reason.INTERFACE_ACTION_QUALITY_PROFILE_REQUIRED":
    "Выберите целевой профиль качества для шага.",
  "error.reason.INTERFACE_ACTION_SEARCH_PARAMETERS_INVALID":
    "Шаг поиска принимает только условие поиска.",
  "error.reason.INTERFACE_ACTION_SEARCH_CONDITION_REQUIRED":
    "Выберите условие поиска для шага.",
  "error.reason.INTERFACE_ACTION_TAG_PARAMETERS_INVALID":
    "Шаг изменения тегов принимает только теги.",
  "error.reason.INTERFACE_ACTION_TAGS_REQUIRED": "Выберите теги для шага.",
  "error.reason.INTERFACE_ACTION_PARAMETERS_UNEXPECTED":
    "Этот шаг действия не принимает параметры.",
  "error.reason.INTERFACE_REQUEST_SAMPLE_LEASE_CONFLICT":
    "Выберите срок предоставления доступа для предпросмотра или бессрочный доступ, но не оба варианта.",
  "error.reason.INTERFACE_MINIMUM_AVAILABILITY_INVALID":
    "Выберите доступность: анонсирован, в кинотеатрах или выпущен.",
  "error.reason.INTERFACE_MINIMUM_AVAILABILITY_FACET_UNSUPPORTED":
    "Минимальную доступность можно задать только для фильма.",
  "error.reason.INTERFACE_SEASON_FOLDERS_FACET_UNSUPPORTED":
    "Папки сезонов можно задать только для сериалов или аниме.",
  "error.reason.INTERFACE_RELEASE_NUMBERING_FACET_UNSUPPORTED":
    "Нумерацию релизов можно задать только для сериалов или аниме.",
  "error.reason.INTERFACE_MONITOR_SELECTION_FACET_UNSUPPORTED":
    "Расширенное отслеживание можно настроить только для сериалов или аниме.",
  "error.reason.INTERFACE_MONITOR_SELECTION_REQUIRED":
    "Выберите хотя бы один сезон или фильм сериала для расширенного отслеживания.",
  "error.reason.INTERFACE_MONITOR_SELECTION_MODE_INVALID":
    "Чтобы выбрать сезоны или фильмы сериала, используйте расширенное отслеживание.",
  "error.reason.INTERFACE_METADATA_LANGUAGE_UNSUPPORTED":
    "Выберите поддерживаемый язык метаданных.",
  "error.reason.INTERFACE_SEARCH_OPTIONS_LIMIT_EXCEEDED":
    "Выберите не более 32 параметров поиска.",
  "error.reason.INTERFACE_SEARCH_ALIAS_INVALID":
    "Поисковые псевдонимы должны содержать от 1 до 200 символов без управляющих символов.",
  "error.reason.INTERFACE_ROOT_FOLDER_NOT_FOUND":
    "Выберите настроенную корневую папку библиотеки.",
  "error.reason.INTERFACE_ROOT_FOLDER_LIBRARY_MISMATCH":
    "Выберите корневую папку в библиотеке тайтла.",
  "error.reason.INTERFACE_TITLE_LIBRARY_UNAVAILABLE":
    "Библиотека тайтла недоступна.",
  "error.reason.INTERFACE_SEARCH_LANGUAGE_UNSUPPORTED":
    "Выберите поддерживаемый язык поиска.",
  "error.reason.INTERFACE_CONFIG_JSON_INVALID":
    "Конфигурация должна быть корректным JSON.",
  "error.reason.INTERFACE_NOTIFICATION_JSON_INVALID":
    "Конфигурация уведомлений должна быть корректным JSON.",
  "error.reason.INTERFACE_LOGIN_FORM_REQUIRED":
    "Включите вход через форму, прежде чем включать вход через медиасервер.",
  "error.reason.INTERFACE_OAUTH_SESSION_REQUIRED":
    "Для этой операции используйте сеанс OAuth.",
  "error.reason.INTERFACE_BROWSE_DIRECTORY_MISSING": "Каталог не существует.",
  "error.reason.INTERFACE_BROWSE_DIRECTORY_UNREADABLE":
    "Не удаётся прочитать каталог.",
  "error.reason.INTERFACE_BROWSE_DIRECTORY_UNAVAILABLE":
    "Не удаётся открыть каталог.",
  "error.reason.INTERFACE_RULE_ACTION_INVALID":
    "Недопустимое определение действия обслуживания.",
  "error.reason.INTERFACE_REQUEST_RULE_SAMPLE_INVALID":
    "Недопустимый пример для правила запросов.",
  "error.reason.INTERFACE_ACQUISITION_SCOPE_INVALID":
    "Для автоматического поиска необходим тайтл и, при необходимости, сезон без фильтров раздела «Требуется».",
  "error.reason.INTERFACE_BROWSE_PATH_NOT_ABSOLUTE":
    "Выберите абсолютный путь к каталогу.",
  "error.reason.INTERFACE_BROWSE_PATH_NOT_DIRECTORY": "Выберите каталог.",
  "error.reason.INTERFACE_CATALOG_FILTER_BLANK":
    "Элементы фильтра каталога не могут быть пустыми.",
  "error.reason.INTERFACE_CATALOG_RATING_INVALID":
    "Выберите минимальную оценку от 0 до 10.",
  "error.reason.INTERFACE_CATALOG_TAG_FILTER_INVALID":
    "Выберите допустимые теги каталога.",
  "error.reason.INTERFACE_CATALOG_YEARS_INVALID":
    "Минимальный год не может превышать максимальный.",
  "error.reason.INTERFACE_CONFIG_KEY_REQUIRED": "Введите ключ конфигурации.",
  "error.reason.INTERFACE_CONFIG_NUMBER_INVALID":
    "Введите допустимое конечное число.",
  "error.reason.INTERFACE_CONFIG_VALUE_AMBIGUOUS":
    "Укажите ровно одно значение конфигурации.",
  "error.reason.INTERFACE_CONFIG_VALUE_OUT_OF_RANGE":
    "Значение конфигурации выходит за допустимый диапазон.",
  "error.reason.INTERFACE_CUSTOM_JOB_ID_REQUIRED":
    "Выберите пользовательское задание для запуска.",
  "error.reason.INTERFACE_CUSTOM_JOB_ID_UNEXPECTED":
    "Идентификатор пользовательского задания можно использовать только с пользовательским заданием.",
  "error.reason.INTERFACE_DISCOVERY_FACET_UNSUPPORTED":
    "Выберите поддерживаемый тип медиаконтента.",
  "error.reason.INTERFACE_DISCOVERY_FILTER_BLANK":
    "Элементы фильтра рекомендаций не могут быть пустыми.",
  "error.reason.INTERFACE_DISCOVERY_LIMIT_INVALID":
    "Выберите допустимое число результатов в разделе.",
  "error.reason.INTERFACE_DISCOVERY_RATING_INVALID":
    "Выберите минимальную оценку от 0 до 10.",
  "error.reason.INTERFACE_DISCOVERY_SURFACE_UNSUPPORTED":
    "Выберите поддерживаемый раздел рекомендаций.",
  "error.reason.INTERFACE_DISCOVERY_YEARS_INVALID":
    "Минимальный год не может превышать максимальный.",
  "error.reason.INTERFACE_DOWNLOAD_CLIENT_NOT_FOUND":
    "Не удалось найти клиент загрузки.",
  "error.reason.INTERFACE_DOWNLOAD_CLIENT_PROVIDER_MISMATCH":
    "Выбранный клиент загрузки использует другой тип провайдера.",
  "error.reason.INTERFACE_DOWNLOAD_ID_INVALID":
    "Недопустимый идентификатор загрузки.",
  "error.reason.INTERFACE_HISTORY_EVENT_TYPE_INVALID":
    "Выберите поддерживаемый тип события истории тайтла.",
  "error.reason.INTERFACE_IDEMPOTENCY_KEY_DUPLICATE":
    "У каждого запроса должен быть уникальный ключ идемпотентности.",
  "error.reason.INTERFACE_IDEMPOTENCY_KEY_REQUIRED":
    "Необходим ключ идемпотентности.",
  "error.reason.INTERFACE_IMPORT_LIBRARY_FACET_MISMATCH":
    "Выбранная библиотека содержит другой тип медиаконтента.",
  "error.reason.INTERFACE_IMPORT_MAPPING_DUPLICATE":
    "Каждую исходную корневую папку можно сопоставить только один раз.",
  "error.reason.INTERFACE_IMPORT_MAPPING_FACET_MISMATCH":
    "Тип медиаконтента в сопоставлении несовместим.",
  "error.reason.INTERFACE_IMPORT_MAPPING_INCOMPLETE":
    "Сопоставьте все исходные корневые папки, прежде чем продолжать.",
  "error.reason.INTERFACE_IMPORT_MAPPING_REQUIRED":
    "Подключите источник или добавьте сопоставление.",
  "error.reason.INTERFACE_IMPORT_MAPPING_ROOT_UNKNOWN":
    "Подключитесь заново, чтобы обнаружить исходную корневую папку перед сопоставлением.",
  "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_INCOMPLETE":
    "Для сопоставления источника необходимы ключ и тип источника.",
  "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_MISMATCH":
    "Сопоставление не соответствует подключённому источнику.",
  "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_UNSELECTED":
    "Сопоставление ссылается на невыбранный источник.",
  "error.reason.INTERFACE_IMPORT_PROWLARR_SOURCE_CONFLICT":
    "Выберите один источник Prowlarr.",
  "error.reason.INTERFACE_IMPORT_ROOT_LIBRARY_MISMATCH":
    "Выберите корневую папку, принадлежащую выбранной библиотеке.",
  "error.reason.INTERFACE_IMPORT_SOURCE_NOT_READY":
    "Дождитесь завершения обнаружения источника, прежде чем продолжать.",
  "error.reason.INTERFACE_IMPORT_SOURCE_REQUIRED":
    "Подключите хотя бы один источник Arr или Prowlarr.",
  "error.reason.INTERFACE_IMPORT_URL_REQUIRED":
    "Введите базовый URL экземпляра Arr.",
  "error.reason.INTERFACE_INDEXER_ERROR_LIMIT_INVALID":
    "Выберите число ошибок индексатора от 1 до 100.",
  "error.reason.INTERFACE_INTERACTIVE_SESSION_REQUIRED":
    "Для продолжения войдите в интерактивном сеансе.",
  "error.reason.INTERFACE_LIST_EPISODE_POLICY_INVALID":
    "Выберите одну политику эпизодов для списка сериалов или аниме.",
  "error.reason.INTERFACE_LIST_EXCLUSIONS_INVALID":
    "Выберите допустимые канонические исключения.",
  "error.reason.INTERFACE_LIST_EXCLUSION_TARGET_REQUIRED":
    "Выберите список для исключения.",
  "error.reason.INTERFACE_LIST_FACET_REQUIRED":
    "Выберите тип медиаконтента для этого фильтра списка.",
  "error.reason.INTERFACE_LIST_LANGUAGE_UNSUPPORTED":
    "Выберите поддерживаемый язык оригинала.",
  "error.reason.INTERFACE_LIST_NOT_PUBLIC":
    "Здесь можно подписываться только на общедоступные списки.",
  "error.reason.INTERFACE_LIST_RATING_LIMIT_EXCEEDED":
    "Уменьшите число фильтров минимальной оценки.",
  "error.reason.INTERFACE_LIST_RATING_OUT_OF_RANGE":
    "Выберите минимальную оценку в пределах шкалы источника.",
  "error.reason.INTERFACE_LIST_RATING_SCALE_REQUIRED":
    "Выберите шкалу для фильтра оценки.",
  "error.reason.INTERFACE_LIST_RATING_SOURCE_UNSUPPORTED":
    "Выберите поддерживаемый источник оценок.",
  "error.reason.INTERFACE_LIST_RATING_VALUE_REQUIRED":
    "Введите значение для фильтра оценки.",
  "error.reason.INTERFACE_LOCATION_DESTINATION_INVALID":
    "Выберите путь назначения или корневую папку, но не оба варианта.",
  "error.reason.INTERFACE_LOCATION_TARGET_INVALID":
    "Выберите тайтлы или корневую папку библиотеки.",
  "error.reason.INTERFACE_LOGIN_MFA_CONFLICT":
    "Задайте непротиворечивые требования MFA для этого способа входа.",
  "error.reason.INTERFACE_LOGIN_MFA_REQUIRED":
    "Выберите требование MFA для входа через Jellyfin.",
  "error.reason.INTERFACE_MAINTENANCE_RULE_NOT_FOUND":
    "Не удалось найти правило обслуживания.",
  "error.reason.INTERFACE_METADATA_DATE_INVALID":
    "Введите допустимую дату метаданных.",
  "error.reason.INTERFACE_METADATA_ID_INVALID":
    "Введите допустимый идентификатор метаданных.",
  "error.reason.INTERFACE_NOTIFICATION_CHANNEL_NOT_FOUND":
    "Не удалось найти канал уведомлений.",
  "error.reason.INTERFACE_PROXY_PROTOCOL_UNSUPPORTED":
    "Выберите поддерживаемый протокол прокси.",
  "error.reason.INTERFACE_PROXY_PROVIDER_UNSUPPORTED":
    "Выберите поддерживаемого провайдера прокси.",
  "error.reason.INTERFACE_PROXY_TIMEOUT_INVALID":
    "Время ожидания запроса должно быть положительным.",
  "error.reason.INTERFACE_QUALITY_PROFILE_ID_REQUIRED":
    "Введите идентификатор профиля качества.",
  "error.reason.INTERFACE_QUALITY_PROFILE_NAME_REQUIRED":
    "Введите имя профиля качества.",
  "error.reason.INTERFACE_QUALITY_PROFILE_TIERS_REQUIRED":
    "Добавьте хотя бы один уровень качества.",
  "error.reason.INTERFACE_QUALITY_PROFILE_VALUE_INVALID":
    "Введите допустимое значение профиля качества.",
  "error.reason.INTERFACE_RELEASE_EXTRA_INVALID":
    "Дополнительные данные релиза должны быть объектом JSON.",
  "error.reason.INTERFACE_RELEASE_SEARCH_EPISODE_INCOMPLETE":
    "Выберите и сезон, и эпизод.",
  "error.reason.INTERFACE_RELEASE_SEARCH_MOVIE_SCOPE_INVALID":
    "Поиск фильма сериала не может включать сезон или эпизод.",
  "error.reason.INTERFACE_RELEASE_SEARCH_TITLE_REQUIRED":
    "Выберите тайтл для поиска релизов.",
  "error.reason.INTERFACE_REQUEST_LEASE_CONFLICT":
    "Выберите срок предоставления доступа или бессрочный доступ, но не оба варианта.",
  "error.reason.INTERFACE_RESERVED_TAG":
    "Для изменения зарезервированных тегов используйте параметры тайтла.",
  "error.reason.INTERFACE_RESTORE_SETUP_REQUIRED":
    "Восстановление доступно только до завершения настройки.",
  "error.reason.INTERFACE_RESTORE_UNAVAILABLE":
    "Восстановление недоступно для этой установки.",
  "error.reason.INTERFACE_RESTORE_UPLOAD_ID_INVALID":
    "Повторно проверьте пакет восстановления, прежде чем продолжать.",
  "error.reason.INTERFACE_RESTORE_UPLOAD_INVALID":
    "Загруженный пакет восстановления недопустим.",
  "error.reason.INTERFACE_RESTORE_UPLOAD_NOT_FOUND":
    "Не удалось найти загруженный пакет восстановления. Проверьте пакет ещё раз.",
  "error.reason.INTERFACE_ROOT_FOLDER_REQUIRED": "Выберите корневую папку.",
  "error.reason.INTERFACE_RULE_EXCLUSION_SUBJECT_REQUIRED":
    "Выберите объект для исключений эпизодов или сезонов.",
  "error.reason.INTERFACE_RULE_PREVIEW_TARGET_CONFLICT":
    "Выберите сохранённое правило или несохранённое условие, но не оба варианта.",
  "error.reason.INTERFACE_RULE_PREVIEW_TARGET_REQUIRED":
    "Выберите сохранённое правило или укажите черновик с его действием.",
  "error.reason.INTERFACE_RULE_SCOPE_CONFLICT":
    "Выберите тайтлы или библиотеку, но не оба варианта.",
  "error.reason.INTERFACE_RULE_SCOPE_REQUIRED":
    "Выберите тайтлы или библиотеку для предпросмотра.",
  "error.reason.INTERFACE_RULE_SOURCE_REQUIRED":
    "Введите исходный код правила для предпросмотра этого условия.",
  "error.reason.INTERFACE_RULE_SUBJECT_SELECTION_INVALID":
    "При выборе объектов выбирайте тайтлы, не выбирая библиотеку.",
  "error.reason.INTERFACE_SCRIPT_ACKNOWLEDGEMENT_REQUIRED":
    "Подтвердите запуск встроенного скрипта оболочки.",
  "error.reason.INTERFACE_SCRIPT_NOT_FOUND": "Не удалось найти скрипт.",
  "error.reason.INTERFACE_SCRIPT_PATH_NOT_ABSOLUTE":
    "Путь к скрипту должен быть абсолютным.",
  "error.reason.INTERFACE_SCRIPT_PATH_REQUIRED":
    "Введите путь к файлу скрипта.",
  "error.reason.INTERFACE_SCRIPT_SCHEDULE_INCOMPLETE":
    "Заполните поля, необходимые для этого расписания.",
  "error.reason.INTERFACE_SCRIPT_TRIGGER_IMMUTABLE":
    "Чтобы использовать другой триггер, создайте новый скрипт.",
  "error.reason.INTERFACE_SCRIPT_TYPE_INVALID":
    "Выберите поддерживаемый тип скрипта постобработки.",
  "error.reason.INTERFACE_SUBTITLE_DELETE_CONFIRMATION_REQUIRED":
    "Просмотрите и подтвердите файлы субтитров перед их удалением.",
  "error.reason.INTERFACE_TAG_NOT_FOUND": "Не удалось найти тег тайтла.",
  "error.reason.INTERFACE_TIMESTAMP_INVALID":
    "Введите допустимую временную метку.",
  "error.reason.INTERFACE_UPGRADE_INELIGIBLE":
    "Эта установка не соответствует условиям запрошенного обновления.",
  "error.libraryScanAlreadyRunning": "Сканирование библиотеки уже выполняется.",
  "error.externalImportSessionGone":
    "Срок действия этого сеанса импорта истёк. Подключитесь заново, чтобы продолжить.",
  "error.tagAlreadyExists": "Тег с таким именем уже существует.",
  "error.ruleTargetsPerson":
    "Для использования полей автора запроса в правиле необходимы права управления разрешениями.",
  "error.rulePackUpToDate": "Этот набор правил уже обновлён.",
  "error.maintenanceArmCountMismatch":
    "Число кандидатов изменилось. Ещё раз проверьте и подтвердите текущее число.",
  "error.mfaStepUpRequired":
    "Срок действия проверки настроек истёк. Введите код из приложения-аутентификатора, чтобы продолжить.",
  "error.reauthenticationRequired":
    "Подтвердите свою личность, чтобы продолжить.",
  "error.rateLimited": "Слишком много запросов. Повторите попытку чуть позже.",
  "error.authenticationRequired": "Войдите, чтобы продолжить.",
  "error.internal": "Внутренняя ошибка сервера",
  "error.internalReference": "Внутренняя ошибка сервера. Идентификатор: {{id}}",
  "error.disconnected": "Соединение разорвано",
  "httpRelease.refusal.noAudioResource":
    "Ни один медиаресурс не соответствует требуемым языкам аудио.",
  "httpRelease.refusal.unsupportedAuth":
    "Для этого ресурса необходим вход у провайдера, который загрузчик HTTP выполнить не может.",
  "httpRelease.refusal.unknownContainer":
    "Не удалось определить загруженные данные как поддерживаемый видеоконтейнер. Частично загруженный файл сохранён.",
  "httpRelease.refusal.destinationExists":
    "Объект назначения уже существует. Существующие файлы не изменены.",
  "httpRelease.refusal.publicationFailed":
    "Не удалось безопасно разместить завершённую загрузку в этой файловой системе. Частично загруженный файл сохранён.",
  "httpRelease.refusal.recoveryUnverified":
    "Не удалось проверить соответствие существующего результата этой загрузке. Он сохранён для проверки.",
  "palette.title": "Цветовая палитра",
  "palette.description":
    "Выберите оформление своей учётной записи для всех устройств.",
  "palette.classic": "Классическая",
  "palette.scryerDefault": "Scryer по умолчанию",
  "palette.slate": "Сланец",
  "palette.ocean": "Океан",
  "palette.forest": "Лес",
  "palette.plum": "Слива",
  "palette.ember": "Угли",
  "palette.graphite": "Графит",
  "palette.papyrus": "Папирус",
  "palette.classicMode": "Классическое оформление",
  "palette.choose": "Выбрать цветовую палитру",
  "palette.useAccent": "Использовать акцентный цвет палитры",
  "palette.saveFailed":
    "Не удалось сохранить оформление. Восстановлен предыдущий вариант.",
  "settings.pluginSettings": "Настройки плагинов",
  "settings.pluginSettingsLoadFailed":
    "Не удалось загрузить настройки плагинов.",
  "settings.pluginSettingsLoading": "Загрузка настроек плагинов…",
  "settings.additionalArchivePasswords": "Дополнительные пароли",
  "settings.additionalArchivePasswordsHelp":
    "Один пароль на строку. Используются только тогда, когда архив требует пароль.",
  "settings.pluginSecretSaved":
    "Значение сохранено. Введите новое или удалите сохранённое.",
  "settings.showEnteredPasswords": "Показать введённые пароли",
  "settings.hideEnteredPasswords": "Скрыть введённые пароли",
  "settings.pluginSettingsSaveFailed":
    "Не удалось сохранить настройки плагинов.",
  "settings.regoValidationUnavailable":
    "Проверка недоступна. Проверьте соединение и снова нажмите «Проверить».",
  "settings.regoValidationFailed":
    "Проверка правила не пройдена. Показать ошибки.",
  "settings.regoFileAvailability":
    "Доступно только при оценке импортированного файла с данными о медиаконтенте. При поиске релизов input.file имеет значение null; проверяйте это перед чтением полей.",
  "settings.regoExtraAvailability":
    "Атрибуты, предоставленные индексатором. Ключи зависят от индексатора и не предлагаются в подсказках.",
  "settings.regoSnippetLabel": "Фрагмент кода",
  "settings.regoHelper.blockScore":
    "Вернуть стандартный блокирующий штраф (-10000). Другие составляющие оценки всё ещё могут компенсировать этот штраф.",
  "settings.regoHelper.sizeGib":
    "Преобразовать число байтов в гибибайты (GiB).",
  "settings.regoHelper.langMatches":
    "Сравнить код языка с другим кодом или поддерживаемым псевдонимом языка.",
  "settings.regoHelper.normalizeSource":
    "Нормализовать обозначение источника, например webdl в WEB-DL.",
  "settings.regoHelper.normalizeCodec":
    "Нормализовать обозначение кодека, например x265 в H.265.",
  "settings.regoHelper.lower": "Преобразовать строку в нижний регистр.",
  "settings.regoHelper.upper": "Преобразовать строку в верхний регистр.",
  "settings.regoHelper.count":
    "Подсчитать элементы массива, множества или объекта либо символы строки.",
  "settings.regoHelper.objectGet":
    "Прочитать значение по ключу объекта, возвращая значение по умолчанию при отсутствии ключа. В правилах запросов и обслуживания используйте именованный факт входных данных.",
  "settings.regoHelper.startsWith":
    "Вернуть признак того, что строка начинается с префикса.",
  "settings.regoSnippet.scoreEntry":
    "Добавить именованную корректировку оценки при выполнении условия для релиза.",
  "settings.regoSnippet.tagScore":
    "Добавить корректировку оценки для тега тайтла. Замените заполнители тега и оценки.",
  "settings.regoSnippet.nullableField":
    "Проверить наличие группы релиза перед сравнением её имени в нижнем регистре.",
  "settings.regoSnippet.denyRequest":
    "Отклонить запрос, если его признак контента для взрослых имеет значение true.",
  "settings.regoSnippet.manualRequest":
    "Потребовать ручной проверки, когда число недавних одобрений достигает порога.",
  "settings.regoSnippet.matchTag":
    "Выбрать объект обслуживания по тегу. Это условие не определяет действие.",
  "settings.trustedProxiesTitle":
    "Доверенные прокси для ограничения частоты запросов",
  "settings.trustedProxiesHelp":
    "При определении клиентов для ограничения частоты запросов доверять X-Forwarded-For только от этих адресов прокси. Это не меняет правила входа, локальный доступ или исключения из ограничения частоты. Изменения применяются сразу.",
  "settings.trustedProxiesAddresses":
    "Адреса IPv4, IPv6 или CIDR (по одному на строку или через запятую)",
  "settings.trustedProxiesEmptyHelp":
    "Если сохранить пустой список, ни один прокси не будет доверенным. Сохранённые настройки имеют приоритет над SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS.",
  "settings.trustedProxiesSavedSource": "Источник: сохранённые настройки",
  "settings.trustedProxiesEnvironmentSource":
    "Источник: окружение (по умолчанию доверенных прокси нет)",
  "settings.trustedProxiesReset":
    "Использовать окружение / значение по умолчанию",
  "settings.trustedProxiesSaved": "Политика доверенных прокси применена.",
  "settings.trustedProxiesLoadError":
    "Не удалось загрузить настройки доверенных прокси.",
  "settings.trustedProxiesSaveError":
    "Не удалось сохранить настройки доверенных прокси.",
  "mediaInfo.discIntactImport":
    "Образ диска импортируется одним файлом. Явно сопоставьте каждый записанный на диске тайтл с эпизодами.",
  "mediaInfo.discReviewRequired": "Результаты анализа диска требуют проверки",
  "mediaInfo.discImportMovie": "Импортировать фильм",
  "mediaInfo.discSkip": "Пропустить (не назначено)",
  "mediaInfo.discAutomaticChoice": "Автоматически выбрать самый длинный тайтл",
  "mediaInfo.discNoTitles":
    "Поддерживаемые тайтлы на диске не найдены. Исходный файл останется на месте.",
  "label.moreInfo": "Подробнее",
  "series.interactiveSearchSeason": "Поиск сезона",
  "help.seasonInteractiveSearchTooltip":
    "Выполнить поиск по всему этому сезону сейчас и выбрать релиз для добавления в очередь.",
  "label.retry": "Повторить",
  "label.account": "Учётная запись",
  "label.advanced": "Расширенные",
  "label.custom": "Пользовательский",
  "label.filterOptions": "Параметры фильтра",
  "label.noMatchingOptions": "Нет подходящих вариантов",
  "label.duration": "Длительность",
  "label.output": "Вывод",
  "label.multiEpisodePack": "Пакет из нескольких эпизодов",
  "label.multiEpisodePackHint":
    "Этот релиз содержит не только эпизод, который вы искали.",
  "nav.dashboard": "Панель управления",
  "nav.group.overview": "Обзор",
  "dashboard.commandDescription": "Обзор операций",
  "dashboard.greetingMorning": "Доброе утро, {{name}}",
  "dashboard.greetingAfternoon": "Добрый день, {{name}}",
  "dashboard.greetingEvening": "Добрый вечер, {{name}}",
  "dashboard.attentionCount": "Требуют внимания: {{count}}",
  "dashboard.attentionCountOne": "1 элемент требует внимания",
  "dashboard.allClear": "Всё работает без проблем.",
  "dashboard.sourceRequests": "запросы",
  "dashboard.sourceImports": "импорт",
  "dashboard.sourcePlugins": "плагины",
  "dashboard.sourceIndexers": "индексаторы",
  "dashboard.pluginUpdateCount": "Обновления плагинов: {{count}}",
  "dashboard.pluginUpdateCountOne": "1 обновление плагина",
  "dashboard.pluginBreaking": "Несовместимые изменения",
  "dashboard.pluginUpdateAll": "Обновить все",
  "dashboard.pluginManage": "Управление",
  "dashboard.statLibrary": "Библиотека",
  "dashboard.statWindow": "24 ч",
  "dashboard.metricMovies": "фильмы",
  "dashboard.metricSeries": "сериалы",
  "dashboard.metricAnime": "аниме",
  "dashboard.metricQueued": "в очереди",
  "dashboard.metricUpgrades": "улучшено",
  "dashboard.metricImported": "импортировано",
  "dashboard.metricFailed": "с ошибкой",
  "dashboard.deltaHint": "Изменение по сравнению с предыдущими 24 часами",
  "dashboard.manualImports": "Ручной импорт",
  "dashboard.recentlyImported": "Недавно импортировано",
  "dashboard.storage": "Хранилище",
  "dashboard.activeQueue": "Активная очередь",
  "dashboard.viewAll": "Все",
  "dashboard.approveRequest": "Одобрить {{name}}",
  "dashboard.dismissRequest": "Отклонить {{name}}",
  "dashboard.upgradeBadge": "Улучшение",
  "dashboard.newImportBadge": "Новый импорт",
  "dashboard.emptyRequests": "Нет ожидающих запросов.",
  "dashboard.emptyImports": "Нет импортов, требующих внимания.",
  "dashboard.emptyImported": "Пока ничего не импортировано.",
  "dashboard.emptyIndexers": "Индексаторы не настроены.",
  "dashboard.emptyClients": "Клиенты загрузки не настроены.",
  "dashboard.emptyStorage": "Корневые папки не настроены.",
  "dashboard.emptyQueue": "Ничего не загружается.",
  "dashboard.columnIndexer": "Индексатор",
  "dashboard.columnSearch": "Поиск",
  "dashboard.columnGrab": "Получение",
  "dashboard.columnFail": "Сбой",
  "dashboard.columnQuota": "Квота",
  "dashboard.columnStatus": "Статус",
  "dashboard.columnClient": "Клиент",
  "dashboard.columnActive": "Активные",
  "dashboard.columnQueue": "Очередь",
  "dashboard.healthCount": "Исправны: {{healthy}}/{{total}}",
  "dashboard.erroringCount": "С ошибками: {{count}}",
  "dashboard.erroringCountOne": "1 с ошибкой",
  "dashboard.downCount": "Недоступны: {{count}}",
  "dashboard.downCountOne": "1 недоступен",
  "dashboard.unmetered": "Без ограничений",
  "dashboard.statusOk": "В норме",
  "dashboard.statusError": "Ошибка",
  "dashboard.rootCount": "Корневые папки: {{count}}",
  "dashboard.rootCountOne": "1 корневая папка",
  "dashboard.storageFree": "Свободно: {{size}}",
  "dashboard.storageUnavailable": "Данные об использовании недоступны",
  "dashboard.storageUsage": "{{used}}/{{total}} ТБ",
  "dashboard.storageLegend": "В норме · 65%+ · 80%+ · 90%+",
  "dashboard.tagLow": "МАЛО",
  "dashboard.tagCrit": "КРИТ",
  "dashboard.queueCount": "Активны: {{count}}",
  "dashboard.queueCountOne": "1 активная",
  "dashboard.columnType": "Тип",
  "dashboard.requestedBy": "Запросили: {{names}}",
  "dashboard.panel.stats": "Краткая сводка",
  "dashboard.panel.statsDescription":
    "Размер библиотеки, получение релизов и импорт за последние 24 часа.",
  "dashboard.panel.requestsDescription":
    "Запросы, ожидающие одобрения, начиная с самых старых.",
  "dashboard.panel.manualImportsDescription":
    "Загрузки, которые не удалось импортировать автоматически.",
  "dashboard.panel.recentlyImportedDescription":
    "Последние тайтлы и эпизоды, добавленные в библиотеку.",
  "dashboard.panel.indexersDescription":
    "Состояние индексаторов, поиск, получение релизов и квота API.",
  "dashboard.panel.downloadClientsDescription":
    "Состояние клиентов загрузки и текущая активность.",
  "dashboard.panel.storageDescription":
    "Используемое место в каждой корневой папке.",
  "dashboard.panel.activeQueueDescription":
    "Текущие загрузки и оставшееся время.",
  "dashboard.layout.edit": "Редактировать панель управления",
  "dashboard.layout.editing": "Редактирование панели управления",
  "dashboard.layout.editHint":
    "Перетаскивайте блоки для перемещения, а их края или углы — для изменения размера.",
  "dashboard.layout.editHintStacked":
    "Перетаскивайте блоки, чтобы изменить их порядок.",
  "dashboard.layout.panels": "Блоки",
  "dashboard.layout.reset": "Восстановить по умолчанию",
  "dashboard.layout.resetConfirmTitle": "Сбросить панель управления?",
  "dashboard.layout.resetConfirmDescription":
    "Для всех размеров экрана будет восстановлена стандартная компоновка. Ваша сохранённая компоновка будет удалена.",
  "dashboard.layout.unsavedTitle": "Несохранённые изменения панели управления",
  "dashboard.layout.unsavedDescription":
    "Вы изменили расположение блоков панели управления, но не сохранили его. Выйти и отменить эти изменения?",
  "dashboard.layout.trayTitle": "Блоки панели управления",
  "dashboard.layout.trayDescription":
    "Перетащите блок на панель управления или добавьте его в конец.",
  "dashboard.layout.placed": "На панели управления",
  "dashboard.layout.addPanel": "Добавить {{name}}",
  "dashboard.layout.dragPanel": "Перетащить {{name}} на панель управления",
  "dashboard.layout.movePanel": "Переместить {{name}}",
  "dashboard.layout.removePanel": "Убрать {{name}}",
  "dashboard.layout.empty":
    "На панели управления нет блоков. Откройте раздел «Блоки», чтобы добавить их.",
  "dashboard.layout.saved": "Компоновка панели управления сохранена.",
  "dashboard.layout.saveFailed":
    "Не удалось сохранить компоновку панели управления.",
  "dashboard.layout.resetDone":
    "Восстановлена стандартная компоновка панели управления.",
  "dashboard.layout.resetFailed":
    "Не удалось сбросить компоновку панели управления.",
  "dashboard.layout.refusal.columnsOutOfRange":
    "В этой компоновке используется число столбцов, которое сервер не принимает.",
  "dashboard.layout.refusal.tooManyPanels":
    "В этой компоновке больше блоков, чем допускает сервер.",
  "dashboard.layout.refusal.panelIdInvalid":
    "В этой компоновке указан блок, который сервер не распознаёт.",
  "dashboard.layout.refusal.panelIdDuplicate":
    "В этой компоновке один и тот же блок размещён дважды.",
  "dashboard.layout.refusal.outsideGrid":
    "Один из блоков этой компоновки находится за пределами сетки.",
  "dashboard.layout.refusal.sizeInvalid":
    "У одного из блоков этой компоновки не задана ширина или высота.",
  "settings.enableFormLogin": "Включить вход",
  "settings.securitySetAdminPasswordTitle": "Задайте пароль администратора",
  "settings.securitySetAdminPasswordDescription":
    "У вашей учётной записи ещё нет пароля. Задайте его сейчас, и Scryer включит вход с этим паролем.",
  "settings.securitySetAdminPasswordAction": "Задать пароль и включить",
  "settings.securitySetAdminPasswordMinLength": "Не менее {{count}} символов.",
  "settings.securityPasswordTooShort":
    "Пароль должен содержать не менее {{count}} символов.",
  "settings.securityAccessControl": "Управление доступом",
  "profile.apiKeyMfaTitle": "Подтвердите личность для создания ключа API",
  "profile.apiKeyMfaDescription":
    "Введите код из приложения-аутентификатора или код восстановления, чтобы продолжить создание ключа API.",
  "profile.apiKeyMfaCode":
    "Код из приложения-аутентификатора или код восстановления",
  "settings.indexerClients": "Управление клиентами",
  "indexerClients.endpoints": "Конечные точки",
  "indexerClients.endpointCopyFailed":
    "Не удалось скопировать URL конечной точки.",
  "indexerClients.existing": "Клиенты индексаторов",
  "indexerClients.none": "Клиентов индексаторов пока нет.",
  "indexerClients.key": "Ключ",
  "indexerClients.protocol.newznab": "Newznab",
  "indexerClients.protocol.torznab": "Torznab",
  "indexerClients.rotateConfirmTitle": "Заменить ключ для {{name}}?",
  "indexerClients.revokeConfirmTitle": "Отозвать ключ для {{name}}?",
  "indexerClients.mfaTitle":
    "Подтвердите личность для изменения клиентов индексаторов",
  "indexerClients.mfaDescription":
    "Введите код из приложения-аутентификатора или код восстановления, чтобы продолжить изменение клиентов индексаторов.",
  "indexerClients.secretOnce":
    "Сохраните этот ключ сейчас. Он больше не будет показан.",
  "indexerClients.apiKey": "Ключ API",
  "indexerClients.keyActive": "Ключ активен",
  "indexerClients.keyRevoked": "Ключ отозван",
  "indexerClients.sourceCount": "Доступных индексаторов: {{count}}",
  "indexerClients.lastUsed": "Последнее использование",
  "indexerClients.never": "Никогда",
  "indexerClients.outcome.caps": "Ответ о возможностях подготовлен",
  "indexerClients.outcome.search": "Ответ на поиск подготовлен",
  "indexerClients.outcome.partial": "Частичный ответ на поиск подготовлен",
  "indexerClients.outcome.artifact": "Загрузка получена",
  "indexerClients.outcome.refused": "Запрос отклонён",
  "indexerClients.outcome.unavailable": "Запрос временно недоступен",
  "indexerClients.outcome.failed": "Ошибка запроса",
  "indexerClients.outcome.unknown": "Результат недоступен",
  "indexerClients.edit": "Редактировать клиент",
  "indexerClients.rotate": "Заменить ключ",
  "indexerClients.revoke": "Отозвать ключ",
  "indexerClients.create": "Создать клиент",
  "indexerClients.name": "Имя клиента",
  "indexerClients.protocols": "Разрешённые протоколы",
  "indexerClients.grants": "Доступные индексаторы",
  "indexerClients.allCategories": "Разрешить все категории этого индексатора",
  "indexerClients.grantAll": "Предоставить доступ ко всем индексаторам",
  "indexerClients.grantIndexer": "Индексатор",
  "indexerClients.granted": "Доступ предоставлен",
  "indexerClients.grantAllCategories": "Все категории",
  "indexerClients.enableRequiresKey":
    "Замените ключ перед включением этого клиента.",
  "indexerClients.error.enableRequiresKey":
    "Ключ этого клиента отозван. Замените ключ перед его включением.",
  "indexerClients.categories": "Разрешённые категории",
  "indexerClients.noCategories":
    "Категории не выбраны: этот индексатор не вернёт клиенту никаких результатов.",
  "indexerClients.save": "Сохранить клиент",
  "indexerClients.keyChangeWarning":
    "Текущий ключ и ссылки получения с ним немедленно перестанут работать. Укажите новый ключ в используемом инструменте, чтобы восстановить доступ.",
  "indexerClients.error.session":
    "Войдите в свою учётную запись для управления клиентами индексаторов. Пропуск входа для локальных IP не распространяется на ключи клиентов.",
  "indexerClients.error.permissions":
    "Для управления клиентами индексаторов необходимы все разрешения приложения.",
  "indexerClients.error.key":
    "Ключ клиента недействителен, отозван или отключён.",
  "indexerClients.error.unavailable":
    "Управление клиентами индексаторов недоступно.",
  "indexerClients.error.missing":
    "Этот клиент больше не существует. Обновите список клиентов.",
  "indexerClients.error.changed":
    "Этот клиент изменился. Обновите список клиентов и откройте его заново перед сохранением.",
  "indexerClients.error.name":
    "Введите имя клиента длиной от 1 до 120 байт без управляющих символов.",
  "indexerClients.error.grant":
    "Выберите существующие отдельные индексаторы, каждый не более одного раза, с допустимыми ограничениями категорий.",
  "indexerClients.error.revision": "Лимит ревизий этого клиента исчерпан.",
  "settings.proxyTestTitle": "Проверить {{name}}",
  "settings.proxyTestUrl": "URL для проверки",
  "settings.proxyTestDescription":
    "Получить этот URL через сохранённый прокси и показать публичный IP, сообщённый конечной точкой.",
  "settings.proxyTestSolverDescription":
    "Попросить сервис решения проверок получить этот URL и показать его исходящий IP. Это проверяет получение данных, но не подтверждает решение проверок или маршрут остального трафика Scryer.",
  "settings.proxyTestRunning": "Проверка исходящего соединения…",
  "settings.proxyTestExitIp": "Выходной IP-адрес прокси",
  "settings.proxyTestSolverIp": "Выходной IP-адрес сервиса обхода защиты",
  "settings.proxyTestNoIp": "Ответ не содержит распознаваемого IP-адреса.",
  "settings.proxyEndpointHelpHttp3":
    "https://host:port (по умолчанию 443). Использует HTTP/3 CONNECT поверх QUIC с проверкой TLS. Назначьте индексатор для проверки маршрута.",
  "settings.libraryEffectiveValue": "Действующее значение: {{value}}",
  "settings.searchLanguages": "Языки поиска релизов",
  "settings.searchLanguagesHelp":
    "Выберите языки в порядке поиска. Автоматический поиск использует эти названия, если предыдущие запросы не нашли подходящий релиз; интерактивный поиск использует все выбранные названия. Языки метаданных и звука не изменяются.",
  "settings.searchAliases": "Собственные названия для поиска релизов",
  "settings.searchAliasesHelp": "Разделяйте названия точками с запятой",
  "settings.libraryMetadataLanguageLabel": "Язык метаданных",
  "settings.libraryEffectiveMetadataLanguage":
    "Действующее значение: {{value}}",
  "settings.librarySeasonFoldersLabel": "Папки сезонов",
  "settings.libraryEffectiveSeasonFolders": "Действующее значение: {{value}}",
  "settings.libraryInheritGlobal":
    "Наследовать глобальное значение по умолчанию",
  "settings.useSeasonFoldersLabel": "Использовать папки сезонов",
  "settings.useSeasonFoldersHelp":
    "Размещать файлы эпизодов в папке соответствующего сезона или спецвыпусков.",
  "jobs.action.runArtworkNow": "Запустить сейчас (вне расписания)",
  "jobs.builtin.title": "Расписание заданий",
  "jobs.custom.title": "Ваши задания",
  "jobs.custom.description":
    "Скрипты, которые вы запускаете по расписанию или по запросу.",
  "jobs.custom.add": "Добавить задание",
  "jobs.custom.addTitle": "Добавить задание",
  "jobs.custom.editTitle": "Изменить задание",
  "jobs.custom.editorDescription":
    "Напишите скрипт или укажите путь к нему на диске, затем выберите время запуска.",
  "jobs.custom.empty":
    "Заданий пока нет. Добавьте скрипт для запуска по расписанию.",
  "jobs.custom.edit": "Изменить задание",
  "jobs.custom.enable": "Включить задание",
  "jobs.custom.disable": "Отключить задание",
  "jobs.custom.disabled": "Отключено",
  "jobs.custom.delete": "Удалить задание",
  "jobs.custom.capturedOutput": "Сохранённый вывод",
  "jobs.custom.noOutput": "Запусков с сохранённым выводом пока нет.",
  "jobs.custom.started": "Начало",
  "jobs.custom.duration": "Длительность",
  "jobs.custom.exitCode": "Код завершения",
  "settings.movieAvailability": "Доступность фильмов",
  "settings.libraryMovieAvailabilityDescription":
    "Переопределите настройки доступности раздела «Фильмы» для этой библиотеки. Каждый фильм по-прежнему может иметь собственный минимальный порог доступности.",
  "settings.movieAvailabilityFacetDescription":
    "Когда автоматический поиск начинает искать фильм. Каждая библиотека фильмов может переопределить эти настройки, а каждый фильм — задать собственный минимальный порог доступности.",
  "settings.movieDefaultAvailability":
    "Минимальный порог доступности по умолчанию",
  "settings.movieDefaultAvailabilityHelp":
    "Применяется ко всем фильмам, для которых не задан собственный минимальный порог доступности.",
  "settings.movieReleaseMarket": "Рынок выпуска",
  "settings.movieReleaseMarketHelp":
    "Двухбуквенный код страны ISO-3166 для выбора дат выхода в кинотеатрах, в цифровом виде и на физических носителях.",
  "settings.movieAvailabilityDelay": "Смещение даты доступности (дни)",
  "settings.movieAvailabilityDelayHelp":
    "Отрицательные значения разрешают получение заранее, положительные откладывают его. При нуле поиск начинается в дату выхода.",
  "settings.movieAvailabilityStatus.available": "Доступен",
  "settings.movieAvailabilityStatus.waiting": "Ожидание",
  "settings.movieAvailabilityStatus.unknown": "Неизвестно",
  "settings.movieAvailability.known": "Известная дата",
  "settings.movieAvailability.estimated": "Расчётная дата",
  "settings.movieAvailability.reason.available": "Выбранный порог достигнут.",
  "settings.movieAvailability.reason.release_dates_unknown":
    "Даты выхода пока неизвестны.",
  "settings.movieAvailability.reason.release_market_refresh_pending":
    "Обновляются даты для выбранного рынка.",
  "settings.movieAvailability.reason.theatrical_release_date_unknown":
    "Дата выхода в кинотеатрах неизвестна.",
  "settings.movieAvailability.reason.home_release_date_unknown":
    "Дата выхода в цифровом виде или на физических носителях неизвестна.",
  "settings.movieAvailability.reason.home_release_date_unusable":
    "Не удалось прочитать известную дату выхода для домашнего просмотра.",
  "settings.movieAvailability.reason.home_release_date_estimated_worldwide":
    "Рассчитано по самой ранней дате выхода в цифровом виде или на физических носителях на любом рынке.",
  "settings.movieAvailability.reason.home_release_date_estimated_from_worldwide_theatrical":
    "Рассчитано как 90 дней после самой ранней даты выхода в кинотеатрах на любом рынке.",
  "settings.movieAvailability.reason.theatrical_release_date_estimated_worldwide":
    "Рассчитано по самой ранней дате выхода в кинотеатрах на любом рынке.",
  "settings.movieAvailability.reason.theatrical_release_date_estimated_from_worldwide_home":
    "Рассчитано по самой ранней дате выхода для домашнего просмотра на любом рынке.",
  "settings.movieAvailability.reason.waiting_for_release_date":
    "Ожидание даты выхода.",
  "settings.movieAvailability.reason.effective_release_date_out_of_range":
    "Итоговая дата выхода вне допустимого диапазона.",
  "settings.seedingProfiles": "Профили сидирования",
  "settings.seedingProfileExisting": "Существующие профили сидирования",
  "settings.seedingProfileNone":
    "Профили сидирования не настроены. Торренты добавляются в загрузку без целей сидирования Scryer и подчиняются собственным ограничениям клиента загрузки.",
  "settings.seedingProfileCreate": "Создать профиль",
  "settings.seedingProfileCreateNew": "Создать профиль сидирования",
  "settings.seedingProfileEdit": "Изменить профиль",
  "settings.seedingProfileConfirmDiscardTitle":
    "Отменить текущие изменения профиля сидирования?",
  "settings.seedingProfileConfirmDiscardDescription":
    "Текущие изменения профиля сидирования будут потеряны.",
  "settings.seedingProfileDeleteConfirm": "Удалить этот профиль сидирования?",
  "settings.seedingProfileNameLabel": "Название",
  "settings.seedingProfileNamePlaceholder":
    "Например: приватный трекер — коэффициент 1.0",
  "settings.seedingProfileGoalPlaceholder":
    "Оставьте пустым для использования собственного ограничения клиента",
  "settings.seedingProfileRatioLabel": "Коэффициент отдачи",
  "settings.seedingProfileRatioHelp":
    "Прекратить сидирование при достижении этого коэффициента отдачи. Если поле пустое, используется собственное ограничение клиента загрузки.",
  "settings.seedingProfileSeedTimeFormatHint":
    "Длительность, например 90m, 36h, 1d 12h или 2w. Число без единиц означает минуты.",
  "settings.seedingProfileSeedTimeLabel": "Время сидирования",
  "settings.seedingProfileMinimumSeedersLabel": "Минимум сидов",
  "settings.seedingProfileMinimumSeedersHelp":
    "Для добавления в загрузку релизов из индексатора с этим профилем требуется не меньше указанного числа сидов. Оставьте пустым для системного значения по умолчанию; введите 0 для отключения проверки.",
  "settings.seedingProfileMinimumSeedersUnknownHelp":
    "Индексаторы, не сообщающие число сидов, всегда допускаются.",
  "settings.seedingProfileMinimumSeedersRoutingHelp":
    "Минимум из профиля, назначенного только через правило маршрутизации клиента загрузки, не применяется: проверка выполняется до выбора клиента, поэтому используется профиль индексатора, затем профиль, импортированный из Prowlarr, затем профиль по умолчанию.",
  "settings.seedingProfileMinimumSeedersInheritHelp":
    "Пустое поле наследует настройку «Минимум сидов по умолчанию», а не значение, импортированное из Prowlarr: назначение этого профиля индексатору под управлением Prowlarr заменяет импортированный минимум.",
  "settings.seedingProfileMinimumSeedersPlaceholder":
    "Оставьте пустым для системного значения по умолчанию",
  "settings.seedingProfileMinimumSeedersOffBadge": "Проверка сидов отключена",
  "settings.seedingProfileMinimumSeedersBadge": "Сидов: ≥ {{value}}",
  "settings.seedingMinimumSeedersFloorLabel": "Минимум сидов по умолчанию",
  "settings.seedingMinimumSeedersFloorHelp":
    "Применяется, если индексатор, из которого получен релиз, не охвачен профилем сидирования. Введите 0, чтобы повсеместно разрешить релизы без сидов.",
  "settings.seedingMinimumSeedersFloorSaved":
    "Минимум сидов по умолчанию сохранён.",
  "settings.seedingProfileSeedTimeHelp":
    "Прекратить сидирование, когда торрент раздаётся указанное время. Если поле пустое, используется собственное ограничение клиента загрузки.",
  "settings.seedingProfileSeedTimeTransmissionHelp":
    "В Transmission нет ограничения общего времени сидирования, поэтому там используется ограничение времени простоя.",
  "settings.seedingProfileBehaviorColumn": "Поведение",
  "settings.seedingProfileSeasonPackSummary": "Пакеты сезонов: {{value}}",
  "settings.seedingProfileIgnoresMinimums": "Игнорирует минимумы трекера",
  "settings.seedingProfileNoneAvailable":
    "Профилей пока нет — создайте профиль в разделе «Настройки → Профили сидирования»",
  "settings.seedingProfileSeasonPackAdvanced":
    "Переопределения для пакетов сезонов (расширенные)",
  "settings.seedingProfileSeasonPackModeLabel": "Цели для пакетов сезонов",
  "settings.seedingProfileSeasonPackModeHelp":
    "При наследовании указанные выше цели используются и для пакетов сезонов. При переопределении вместо них применяются цели для пакетов сезонов.",
  "settings.seedingProfileSeasonPackInherit": "Наследовать",
  "settings.seedingProfileSeasonPackOverride": "Переопределить",
  "settings.seedingProfileSeasonPackRatioLabel":
    "Коэффициент отдачи для пакетов сезонов",
  "settings.seedingProfileSeasonPackSeedTimeLabel":
    "Время сидирования для пакетов сезонов",
  "settings.seedingProfileSeasonPackGoalsHelp":
    "Используется только в режиме переопределения. Пустое поле передаёт соответствующую цель под управление клиента загрузки.",
  "settings.seedingProfileHonorTrackerMinimumsLabel":
    "Учитывать минимумы трекера",
  "settings.seedingProfileHonorTrackerMinimumsHelp":
    "Повышать итоговые цели до минимального коэффициента отдачи и времени сидирования, указанных релизом, чтобы приватные трекеры не фиксировали нарушение hit and run.",
  "settings.seedingProfilePostImportTrackingLabel": "После импорта",
  "settings.seedingProfilePostImportTrackingHelp":
    "Продолжает ли Scryer управлять торрентом после импорта файлов.",
  "settings.seedingProfilePostImportTrackingPark":
    "Продолжать управление до достижения целей",
  "settings.seedingProfilePostImportTrackingHandOff":
    "Передать управление клиенту загрузки",
  "settings.seedingProfilePostImportTrackingHandOffBadge":
    "Управление передано",
  "settings.seedingProfilePostImportTrackingHandOffHelp":
    "После импорта Scryer прекращает управление этим торрентом: запись остаётся в клиенте, никогда не удаляется автоматически и исчезает из очереди. Цели сидирования и действие при их достижении больше не применяются — действуют собственные ограничения клиента. Во время сидирования файлы по-прежнему импортируются через жёсткие ссылки или копирование.",
  "settings.seedingProfileGoalMetActionLabel": "При достижении цели",
  "settings.seedingProfileGoalMetActionHelp":
    "Что происходит с торрентом после достижения цели сидирования.",
  "settings.seedingProfileGoalMetRemoveEntry": "Удалить запись",
  "settings.seedingProfileGoalMetStopSeeding": "Прекратить сидирование",
  "settings.seedingProfileGoalMetKeep": "Продолжить сидирование",
  "settings.seedingProfileNeverRemoveLabel": "Никогда не удалять",
  "settings.seedingProfileNeverRemoveHelp":
    "Запрещает любое автоматическое удаление торрентов, добавленных в загрузку с этим профилем, даже после достижения цели. Их придётся удалять вручную.",
  "settings.seedingProfileNeverRemoveBadge": "Никогда не удаляется",
  "settings.seedingProfileDefaultTitle": "Профиль сидирования по умолчанию",
  "settings.seedingProfileDefaultLabel":
    "Применяется, если нет более конкретного соответствия",
  "settings.seedingProfileDefaultHelp":
    "Используется при добавлении торрентов в загрузку, если профиль не указан ни у индексатора, ни в правиле маршрутизации клиента загрузки.",
  "settings.seedingProfileDefaultNone":
    "Без профиля по умолчанию — использовать собственные ограничения клиента",
  "settings.seedingProfileDefaultBadge": "По умолчанию",
  "settings.seedingProfileMissing": "Профиль «{{id}}» отсутствует",
  "settings.seedingProfileSaved": "Профиль сидирования сохранён.",
  "settings.seedingProfileSaveError":
    "Не удалось сохранить профиль сидирования.",
  "settings.seedingProfileDeleted": "Профиль сидирования удалён.",
  "settings.seedingProfileDeleteError":
    "Не удалось удалить профиль сидирования.",
  "settings.seedingProfileDefaultSaved":
    "Профиль сидирования по умолчанию сохранён.",
  "settings.seedingProfileProwlarrManaged": "Под управлением Prowlarr",
  "settings.seedingProfileProwlarrManagedMinimum":
    "Под управлением Prowlarr · минимум сидов: {{count}}",
  "settings.seedingProfileProwlarrManagedMinimumOne":
    "Под управлением Prowlarr · минимум 1 сид",
  "settings.seedingProfileProwlarrManagedMinimumOff":
    "Под управлением Prowlarr · проверка сидов отключена",
  "settings.seedingProfileInherit": "Наследовать значение по умолчанию",
  "settings.seedingProfileInheritProwlarrMinimum":
    "Наследовать значение по умолчанию · минимум сидов Prowlarr: {{count}}",
  "settings.seedingProfileInheritProwlarrMinimumOne":
    "Наследовать значение по умолчанию · минимум 1 сид Prowlarr",
  "settings.seedingProfileInheritProwlarrMinimumOff":
    "Наследовать значение по умолчанию · проверка сидов Prowlarr отключена",
  "settings.seedingProfileRoutingInherit":
    "Наследовать глобальное значение по умолчанию",
  "settings.seedingProfileColumn": "Профиль сидирования",
  "settings.seedingProfileIndexerLabel": "Профиль сидирования для {{name}}",
  "settings.seedingProfileRoutingLabel": "Профиль сидирования для {{name}}",
  "settings.seedingProfileNotApplicable": "Только для торрент-индексаторов",
  "status.indexerSeedingProfileSaving": "Сохранение профиля сидирования…",
  "status.indexerSeedingProfileSaved":
    "Профиль сидирования для «{{name}}» сохранён.",
  "settings.renameFunctionOptionalGroupTitle": "Необязательная группа",
  "settings.renameOptionalGroupDescription":
    "Выводить группу, только если её условный токен имеет значение. Необязательные группы нельзя вкладывать друг в друга.",
  "settings.renameTokenTitleWithArticle":
    "Название с переносом начального артикля в конец с учётом языка метаданных",
  "settings.renameTokenTitleFirstCharacter":
    "Первая буква или цифра после сортировки артиклей с учётом языка; сохраняет нелатинские символы, при отсутствии подходящего символа используется _",
  "rename.namingLanguage": "Язык именования: {{language}}",
  "rename.organizingFolderChanges":
    "Тайтлов со сменой папки организации: {{count}}",
  "settings.renameValidationInvalidOptionalGroup":
    "Недопустимая необязательная группа",
  "settings.renameValidationNestedOptionalGroup":
    "Необязательные группы нельзя вкладывать друг в друга",
  "settings.renameValidationUnsupportedOptionalFallback":
    "Необязательные группы не поддерживают запасные ветви",
  "rename.noRenamableFiles": "Нет файлов, требующих переименования.",
  "rename.blockedFiles":
    "Не удастся переименовать файлов: {{count}}. Из-за них переименование этого тайтла будет остановлено.",
  "rename.sampleNote":
    "Показаны первые {{shown}} из {{renamable}} файлов для переименования.",
  "settings.apiExplorerLabel": "Включить обозреватель API",
  "settings.apiExplorerHelp":
    "Добавляет GraphiQL в пункт навигации «API» для пользователей с правом управления системными настройками. Запросы выполняются с правами доступа к библиотекам, предоставленными ключом API или OAuth.",
  "apiExplorer.accessMode": "Режим доступа",
  "apiExplorer.renameTab": "Переименовать вкладку",
  "apiExplorer.tabName": "Название вкладки",
  "apiExplorer.apiKey": "Ключ API",
  "apiExplorer.oauth": "Приложение OAuth",
  "apiExplorer.apiKeyHelp":
    "Ваши права доступа к приложению и библиотекам без интерактивного доступа к учётной записи. Подписки недоступны.",
  "apiExplorer.oauthHelp":
    "Только ваши права доступа к библиотекам. Настройки приложения и операции с учётными записями недоступны.",
  "apiExplorer.sessionHelp":
    "Используется текущий сеанс. Учётные данные и настройка клиента не требуются.",
  "theme.label": "Тема",
  "search.addConfigMinAvailabilityNoReleaseDate":
    "Дата выхода пока неизвестна. Автоматический поиск начнётся, когда она станет известна.",
  "search.monitorType.advanced": "Расширенный",
  "monitorSelection.seasons": "Сезоны",
  "monitorSelection.seriesMovies": "Фильмы сериала",
  "monitorSelection.specials": "Спецвыпуски",
  "monitorSelection.season": "Сезон {{number}}",
  "monitorSelection.seasonNamed": "Сезон {{number}}: {{name}}",
  "monitorSelection.selectAll": "Выбрать всё",
  "monitorSelection.clear": "Очистить",
  "monitorSelection.retry": "Повторить",
  "monitorSelection.loading": "Загрузка сезонов…",
  "monitorSelection.loadFailed": "Не удалось загрузить сезоны этого тайтла.",
  "monitorSelection.noSeasons":
    "Для этого тайтла не получено ни одного сезона.",
  "monitorSelection.unavailable":
    "Для расширенного отслеживания требуется соответствие в TVDB, которого у этого результата нет.",
  "monitorSelection.emptyHint":
    "Выберите хотя бы один сезон или фильм сериала.",
  "monitorSelection.summaryPrefix": "Запрошено: {{selection}}",
  "monitorSelection.summaryMovies": "Фильмы: {{movies}}",
  "toast.catalogAdded": "Добавлено в каталог",
  "toast.catalogAddedAutoSearch": "Автоматический поиск поставлен в очередь.",
  "toast.releaseGrabbed": "Релиз добавлен в загрузку",
  "toast.titleImported": "Тайтл импортирован",
  "toast.viewInCatalog": "Открыть в каталоге",
  "label.copy": "Копировать",
  "label.copied": "Скопировано",
  "title.renameAction": "Переименовать",
  "title.topBilledCast": "В главных ролях",
  "title.dubCast": "Актёры дубляжа",
  "title.dubCastLanguage": "Язык",
  "title.clearBlockedRelease": "Убрать {{releaseName}} из списка блокировки",
  "title.clearBlockedReleaseHint":
    "Убрать этот релиз из списка блокировки, чтобы его снова можно было добавить в загрузку",
  "title.bulkRenameTitle": "Переименовать выбранные тайтлы",
  "title.bulkRenameDescription":
    "Просмотрите и примените переименования для выбранных тайтлов ({{count}}).",
  "title.files": "Файлы",
  "pendingImports.searchYearLabel": "Год",
  "pendingImports.searchYearPlaceholder": "Год",
  "pendingImports.searchYearClear": "Очистить год",
  "pendingImports.searchYearHint":
    "Год получен при сканировании. Очистите его, чтобы расширить поиск, если в имени файла указан неверный год.",
  "pendingImports.searchRequestFailed":
    "Не удалось выполнить поиск метаданных: {{error}}",
  "pendingImports.alreadyInLibrary": "Уже в этой библиотеке",
  "pendingImports.attachToExistingTitle": "Привязать к существующему тайтлу",
  "pendingImports.attachSuccess": "Ожидающий импорт привязан к {{name}}.",
  "pendingImports.reason.invalid_title_folder_ownership":
    "Папка тайтла требует восстановления",
  "pendingImports.invalidOwnershipHelp":
    "За этим тайтлом ошибочно закреплена корневая папка библиотеки или её родительская папка. Проверьте восстановление папки тайтла в разделе «Активность». Исходные файлы сохранены; не перемещайте файлы в записанную корневую папку.",
  "pendingImports.reason.metadata_id_lookup_unresolved":
    "Поиск метаданных не смог определить внешний идентификатор этой папки; попытка повторится при следующем сканировании",
  "nzb.currentFile": "Текущий файл",
  "nzb.movieAvailabilityWarning": "Автоматический поиск ожидает: {{reason}}",
  "nzb.recommended": "Рекомендуется",
  "activity.activityFilter.warning": "Предупреждение",
  "queue.state.importedSeeding": "Импортирован · Сидирование",
  "queue.state.waitingForDiskSpace": "Ожидание места на диске",
  "activity.refreshTimedOut": "Время ожидания обновления истекло",
  "activity.refreshBehind":
    "Обновление очереди не отразило последние изменения. Повторите попытку.",
  "activity.retry": "Повторить",
  "activity.loadMore": "Загрузить ещё",
  "activity.staleData":
    "Показан результат последнего успешного запроса. Обновить данные не удалось.",
  "activity.countUnknown":
    "Число импортов, требующих внимания, пока недоступно",
  "activity.countStale":
    "Число импортов, требующих внимания, может быть устаревшим",
  "queue.state.warning": "Предупреждение",
  "queue.seeding.stateSeeding": "Сидирование",
  "queue.seeding.stateGoalMet": "Цель сидирования достигнута",
  "queue.seeding.stateHeldPrivate": "Сидирование · приватный",
  "queue.seeding.stateNeverRemove": "Бессрочное сидирование",
  "queue.seeding.ratio": "Коэффициент отдачи: {{value}}",
  "queue.seeding.seedTime": "Время сидирования: {{value}}",
  "queue.seeding.private": "Приватный трекер",
  "queue.seeding.privateTooltip":
    "Приватный трекер. Scryer никогда не удаляет этот торрент из клиента самостоятельно, если не достигнута цель профиля сидирования.",
  "queue.assignTitleSearchHint": "Введите название тайтла для поиска…",
  "queue.passwordRetryAccepted": "Пароль обновлён, повторная попытка принята.",
  "queue.passwordRetryRefused":
    "Клиент загрузки отклонил повторную попытку. Загрузка сохранена.",
  "queue.passwordRetryUncertain":
    "Результат повторной попытки неизвестен. Загрузка сохранена до согласования состояния; ещё одна повторная попытка не отправлена.",
  "queue.passwordRetryFailed":
    "Не удалось запустить повторную попытку с паролем. Обновите задание, чтобы проверить его текущее состояние.",
  "queue.belowMinimumSeeders":
    "Сидов меньше минимума, заданного для этого индексатора.",
  "importHistory.showPassword": "Показать пароль",
  "importHistory.hidePassword": "Скрыть пароль",
  "settings.passwordMinLengthError":
    "Пароль должен содержать не менее {{min}} символов.",
  "settings.temporaryPassword": "Временный пароль",
  "settings.resetPassword": "Сбросить пароль",
  "settings.setTemporaryPassword": "Задать временный пароль",
  "settings.temporaryPasswordHelp":
    "После локального входа пользователь должен заменить этот пароль. Передайте его по отдельному безопасному каналу.",
  "settings.passwordChangeRequired": "Требуется смена пароля",
  "settings.temporaryPasswordResetTitle": "Задать временный пароль?",
  "settings.temporaryPasswordResetDescription":
    "Это отзовёт активные сеансы пользователя {{name}} и потребует выбрать новый пароль после локального входа.",
  "settings.advancedConfig": "Расширенные настройки",
  "settings.indexerRateLimits": "Ограничения частоты запросов",
  "settings.arrImportAction": "Импортировать пользовательский формат Arr",
  "settings.ruleAiPrompt": "Запрос для ИИ",
  "settings.ruleAiPromptTitle": "Создать правило с помощью ИИ",
  "settings.ruleAiPromptDescription":
    "Опишите желаемый результат, затем скопируйте запрос в ИИ-помощник.",
  "settings.ruleAiPromptOutcome": "Желаемый результат",
  "settings.ruleAiPromptOutcomePlaceholder":
    "Например: начислять 100 баллов релизам с японской аудиодорожкой.",
  "settings.ruleAiPromptCopy": "Копировать запрос",
  "settings.ruleAiPromptCopied": "Скопировано",
  "settings.arrImportTitle": "Импортировать пользовательские форматы Arr",
  "settings.arrImportDescription":
    "Вставьте экспорт пользовательских форматов Sonarr или Radarr, чтобы создать новый черновик правила Rego.",
  "settings.arrImportSource": "Исходное приложение",
  "settings.arrImportJson": "JSON пользовательских форматов",
  "settings.arrImportJsonPlaceholder":
    "Вставьте один пользовательский формат или массив пользовательских форматов.",
  "settings.arrImportReviewAction": "Проверить форматы",
  "settings.arrImportReview":
    "Готово к преобразованию пользовательских форматов: {{count}}",
  "settings.arrImportNoFormats":
    "В этом JSON не найдено допустимых пользовательских форматов.",
  "settings.arrImportInvalidJson":
    "Вставьте корректный JSON, экспортированный из Sonarr или Radarr.",
  "settings.arrImportReviewFailed":
    "Не удалось проверить эти пользовательские форматы.",
  "settings.arrImportScore": "Баллы",
  "settings.arrImportScoreRequired":
    "Перед преобразованием введите целое число баллов для {{name}}.",
  "settings.arrImportSuggestedScore": "Предлагаемые баллы: {{score}}.",
  "settings.arrImportUseSuggestion": "Использовать предложение",
  "settings.arrImportScoreReasonExported": "Используются баллы из экспорта.",
  "settings.arrImportScoreReasonDefault":
    "Используются баллы по умолчанию из экспорта; ниже доступны другие варианты из профилей.",
  "settings.arrImportScoreReasonPreference":
    "Название формата выражает предпочтение.",
  "settings.arrImportScoreReasonPenalty":
    "Название формата обозначает нечто нежелательное.",
  "settings.arrImportScoreReasonExtras":
    "Судя по всему, формат выявляет дополнительные материалы или образцы.",
  "settings.arrImportScoreSuggestionHelp":
    "Предложения — это начальные значения, которые можно изменить. Баллы совпавших форматов суммируются; штрафы могут компенсироваться другими баллами.",
  "settings.arrImportAmbiguousScore":
    "Для этого формата предложено несколько значений баллов. Выберите нужное.",
  "settings.arrImportTranslate": "Преобразовать в Rego",
  "settings.arrImportTranslationFailed":
    "Преобразование не удалось. Вставленный JSON по-прежнему доступен для проверки или повторной попытки.",
  "settings.arrImportResultSummary":
    "Преобразовано: {{translated}}; отключено для исправления вручную: {{disabled}}.",
  "settings.arrImportDraftName": "Импортированные пользовательские форматы Arr",
  "settings.arrImportDraftDescription":
    "Импортировано из JSON пользовательских форматов Arr.",
  "settings.arrImportDiagnostics": "Примечания к преобразованию",
  "settings.arrImportApply": "Использовать импортированный черновик",
  "settings.refReleaseNormalizedTokens":
    "Упорядоченные нормализованные токены, полученные парсером релиза",
  "settings.refReleaseHasHdrFallback":
    "Истина, если парсер релиза обнаруживает резервный слой HDR",
  "settings.refReleaseStereoscopy":
    "Формат представления, заявленный в названии релиза; null означает, что он неизвестен",
  "settings.refSectionStereoscopy": "— Стереоскопическое представление",
  "settings.refStereoPresentation":
    "Значения two_d, three_d или mixed_2d_3d; это сведения из названия, а не проверенные свойства потоков",
  "settings.refStereoLayout":
    "Значения side_by_side, top_bottom, frame_sequential, row_interleaved, column_interleaved, checkerboard или anaglyph; null, если значение не указано, противоречиво или смешано",
  "settings.refStereoSampling":
    "Значения half или full для расположения рядом или сверху/снизу; null, если неизвестно",
  "settings.refStereoEncoding":
    "Значение mvc при явном подтверждении маркером 3D; null, если неизвестно",
  "settings.refReleaseStereoscopyTokens":
    "Канонические стереотокены: stereo:two_d, stereo:three_d, stereo:mixed_2d_3d, stereo:has_3d, stereo:layout:<layout>, stereo:sampling:half/full, stereo:encoding:mvc и stereo:<field>_conflict. Пустое значение означает отсутствие сведений. Исходные normalized_tokens сохраняют первоначальный смысл.",
  "settings.refReleaseRevision":
    "Числовая ревизия релиза: 0 для оригинала, 1 для PROPER/REPACK/RERIP, 2 для REPACK2 и так далее",
  "settings.refReleaseProtocol":
    "Протокол загрузки: usenet, torrent или http; null, если неизвестен. Фиксируется из записи индексатора для импорта и последующего пересчёта баллов. Для старых файлов остаётся неизвестным, если в записи индексатора не сохранились метаданные протокола.",
  "settings.ruleTestListingProtocol": "Протокол загрузки",
  "settings.ruleTestModeLabel": "Что проверить",
  "settings.ruleTestModeRelease": "Название релиза",
  "settings.ruleTestModeStoredFile": "Проверить сохранённый файл",
  "settings.ruleTestStoredFile": "Сохранённый файл",
  "settings.ruleTestStoredFileHelp":
    "Баллы рассчитываются по размеру самого файла и данным записи индексатора, зафиксированным при добавлении в загрузку.",
  "settings.ruleTestStoredFileSelectTitle":
    "Сначала выберите тайтл из библиотеки",
  "settings.ruleTestStoredFileLoading": "Загрузка файлов…",
  "settings.ruleTestStoredFilePlaceholder": "Выберите файл",
  "settings.ruleTestStoredFileNone": "У этого тайтла или эпизода нет файлов",
  "settings.ruleTestStoredFileFilter": "Фильтровать файлы",
  "settings.ruleTestStoredFileLoadFailed":
    "Не удалось загрузить файлы этого тайтла.",
  "settings.ruleTestListingFacts": "Данные записи индексатора",
  "settings.ruleTestListingFactsHelp":
    "Необязательные данные записи индексатора. Для правил все пустые поля считаются неизвестными (null). Возраст релиза рассчитывается от времени публикации до текущего момента.",
  "settings.ruleTestListingPublishedAt": "Дата публикации",
  "settings.ruleTestListingPublishedAtPlaceholder":
    "2024-01-31, 2024-01-31T12:00:00Z или Wed, 31 Jan 2024 12:00:00 +0000",
  "settings.ruleTestListingThumbsUp": "Положительные голоса",
  "settings.ruleTestListingThumbsDown": "Отрицательные голоса",
  "settings.ruleTestListingPasswordProtected": "Защищён паролем",
  "settings.ruleTestListingUnknown": "Неизвестно",
  "settings.ruleTestListingYes": "Да",
  "settings.ruleTestListingNo": "Нет",
  "settings.ruleTestListingIndexerLanguages":
    "Языки индексатора (через запятую)",
  "settings.ruleTestListingExtra": "Дополнительные атрибуты (объект JSON)",
  "settings.ruleTestListingPublishedAtInvalid":
    "Дата публикации должна быть датой (2024-01-31), временем RFC 3339 с секундами и часовым поясом (2024-01-31T12:00:00Z) или датой RFC 2822 (Wed, 31 Jan 2024 12:00:00 +0000).",
  "settings.ruleTestListingVotesInvalid":
    "Число голосов должно быть целым неотрицательным числом.",
  "settings.ruleTestListingExtraInvalid":
    "Дополнительные атрибуты должны быть объектом JSON.",
  "settings.ruleTestResultRelease": "Оценённый релиз",
  "settings.ruleTestResultListingUnknown":
    "Данные записи индексатора неизвестны: файл найден сканированием, принят под управление или импортирован до начала сохранения снимков записей, поэтому для правил все эти данные имеют значение null.",
  "settings.ruleTestResultAgeDays": "Возраст (дни)",
  "settings.ruleTestResultAgeAtGrab":
    "{{days}} на момент добавления в загрузку",
  "settings.ruleTestResultCapturedAt": "Зафиксировано",
  "settings.refProfileScoringPersona":
    "Итоговый стиль оценки: balanced, audiophile, efficient или compatible",
  "settings.refProfileScoringOverrides":
    "Явно заданные параметры профиля; null оставляет значение стиля оценки по умолчанию без изменений",
  "settings.refSectionScoringOverrides": "— Переопределения оценки",
  "settings.refOverrideX265":
    "Разрешать HEVC не в 4K; null наследует значение стиля оценки по умолчанию",
  "settings.refOverrideDolbyVision":
    "Отклонять Dolby Vision без резервного варианта; null наследует значение стиля оценки по умолчанию",
  "settings.refOverrideCompact":
    "Предпочитать компактные кодировки; null наследует значение стиля оценки по умолчанию",
  "settings.refOverrideLossless":
    "Предпочитать звук без потерь; null наследует значение стиля оценки по умолчанию",
  "settings.refOverrideUpscaled":
    "Штрафовать релизы с повышенным разрешением исходного видео; null наследует значение стиля оценки по умолчанию",
  "settings.refCtxCoverageTotalRuntime":
    "Суммарная длительность в минутах всех элементов, охваченных релизом, если известна",
  "settings.refCtxCoverageMemberRuntime":
    "Длительность в минутах одного из охваченных элементов, если известна",
  "settings.refCtxCoverageMemberCount":
    "Число охваченных элементов, если известно",
  "settings.pluginAutoUpdateEnabled":
    "Автоматически устанавливать патч-обновления плагинов",
  "settings.pluginAutoUpdateEnabledHelp":
    "Если включено, задание обновления каталога плагинов по расписанию (или его ручной запуск) обновляет официальные плагины до последней патч-версии, например 1.0.1 → 1.0.2, и устанавливает оптимизированные сборки установленной версии при их наличии. Минорные и мажорные обновления остаются под управлением администратора.",
  "settings.externalUrl": "Внешний URL",
  "settings.externalUrlHint":
    "URL для доступа из браузера, используемый в ссылках воспроизведения Jellyfin или Emby.",
  "label.watchIn": "Смотреть в",
  "settings.pluginBlocked": "Заблокирован",
  "settings.pluginBlockedNotRunning": "Не запущен",
  "status.pluginAutoUpdateSettingsSaved":
    "Настройка автоматического обновления плагинов сохранена.",
  "form.indexerRateLimitSeconds": "Минимум секунд между запросами",
  "form.indexerRateLimitSecondsPlaceholder": "Без задержки",
  "form.indexerRateLimitSecondsInvalid":
    "Минимум секунд между запросами должен быть целым числом; оставьте поле пустым для отключения задержки.",
  "form.indexerMaxQueriesPerMinute": "Максимум запросов в минуту",
  "form.indexerMaxQueriesPerMinutePlaceholder": "Без ограничений",
  "form.indexerMaxQueriesPerMinuteInvalid":
    "Максимум запросов в минуту должен быть целым числом не меньше 1; оставьте поле пустым для отключения ограничения.",
  "form.subtitleProviderFacetsRequired":
    "Выберите хотя бы один тип содержимого. Scryer никогда не выполняет поиск через провайдера, у которого не выбран ни один тип.",
  "qualityProfile.qualityPreference": "Приоритет качества",
  "qualityProfile.qualityPreferenceHelp":
    "Наиболее предпочтительное качество — вверху. Приоритет уровня качества важнее баллов. Новые уровни добавляются с самым низким приоритетом.",
  "qualityProfile.moveQualityTierUp": "Переместить {{value}} вверх",
  "qualityProfile.moveQualityTierDown": "Переместить {{value}} вниз",
  "qualityProfile.dragQualityTier":
    "Перетащите, чтобы изменить порядок {{value}}",
  "qualityProfile.ignoreProperRepack": "Игнорировать PROPER/REPACK",
  "qualityProfile.ignoreProperRepackInfo":
    "Принимать решение по качеству и баллам без автоматического предпочтения или защиты ревизий. Правила оценки по-прежнему могут начислять баллы релизам PROPER/REPACK.",
  "seasonSection.seasonSearchResults": "Результаты поиска: {{season}}",
  "seriesOverview.selectColumn": "Выбрать",
  "seriesOverview.selectEpisodeForDelete": "Выбрать {{name}}",
  "seriesOverview.selectSeasonForDelete": "Выбрать все эпизоды в {{name}}",
  "seriesOverview.clearEpisodeSelection": "Снять выделение",
  "seriesOverview.deleteSelectedEpisodeFiles": "Удалить выбранные: {{count}}",
  "seriesOverview.deleteEpisodeFilesTitle": "Удалить файлы выбранных эпизодов",
  "seriesOverview.deleteEpisodeFilesDescription":
    "Файлы выбранных эпизодов будут безвозвратно удалены с диска. Записи эпизодов сохранятся.",
  "seriesOverview.deleteEpisodeFilesSummary":
    "Файлов: {{files}}; эпизодов: {{episodes}}",
  "seriesOverview.deleteEpisodeFilesPreviewFailures":
    "Не удалось предварительно проверить файлов: {{count}}. Их удаление может завершиться ошибкой.",
  "mediaFile.reviewRequired": "Требуется проверка",
  "mediaFile.discReviewInventory":
    "Тайтлы из последней проверки. Сохранённый выбор и привязки эпизодов останутся без изменений, пока вы не сохраните проверенный вариант.",
  "scoring.current": "Текущие баллы",
  "scoring.score": "Баллы",
  "scoring.previous": "Предыдущие баллы",
  "scoring.change": "Изменение",
  "scoring.decision": "Решение",
  "scoring.manualOverride": "Ручной выбор",
  "scoring.needsRefresh": "Требуется обновление",
  "scoring.timeSensitive": "Зависит от времени",
  "scoring.breakdown": "Расчёт баллов",
  "scoring.customRule": "Пользовательское правило",
  "scoring.systemRule": "Системное правило",
  "scoring.rejected": "Отклонён",
  "scoring.refreshOne": "Пересчитать баллы",
  "scoring.refreshSelected": "Пересчитать баллы",
  "scoring.refreshFailed":
    "Не удалось пересчитать баллы. Проверьте выбранные элементы и права доступа.",
  "mediaFile.commentary": "Комментарии",
  "mediaFile.audioDescription": "Тифлокомментарий",
  "mediaFile.hearingImpaired": "Для слабослышащих",
  "mediaFile.program": "Программа {{id}}",
  "mediaFile.discTitle": "Тайтл диска для воспроизведения",
  "mediaFile.discReviewAction": "Диск требует внимания",
  "mediaFile.discOutcomeAutomatic":
    "Воспроизводится тайтл {{id}} ({{duration}}), автоматически выбранный как самый длинный тайтл с завершённой проверкой.",
  "mediaFile.discOutcomeManual":
    "Воспроизводится тайтл {{id}} ({{duration}}), выбранный вами. Автоматический выбор для этого диска отключён.",
  "mediaFile.discOutcomeUnselected":
    "Не удалось выбрать тайтл для воспроизведения. Выберите его ниже.",
  "mediaFile.discOutcomeIncomplete":
    "Проверка выбранного тайтла не завершена ({{status}}). Выберите тайтл с завершённой проверкой или проверьте файл повторно.",
  "mediaFile.discChooseTitle": "Какой тайтл воспроизводить",
  "mediaFile.discAutomaticCurrent": "Сейчас выбран тайтл {{id}}",
  "mediaFile.discTitleChapterCount": "Глав: {{count}}",
  "mediaFile.discTitleAudioCount": "Аудиодорожек: {{count}}",
  "mediaFile.discTitleNotSelectable": "Проверка: {{status}}",
  "mediaFile.discTitleMissing": "Больше нет на диске",
  "mediaFile.discPlaying": "Воспроизводится",
  "mediaFile.discUseTitle": "Использовать этот тайтл",
  "mediaFile.discStreamsAndChapters": "Потоки и главы",
  "mediaFile.discTechnicalDetails": "Технические сведения",
  "mediaFile.discTitleIdentity": "Тайтл {{id}}",
  "mediaFile.discInspectTitles": "Проверить потоки и главы тайтла",
  "mediaFile.discAngleInventory":
    "Ракурсов: {{count}}. При проверке медиа анализируется первый ракурс, если он доступен.",
  "mediaFile.discTitleAliases": "Эквивалентные идентификаторы тайтлов: {{ids}}",
  "mediaFile.discTitleBudgetExhausted":
    "При проверке этого тайтла достигнут лимит анализа.",
  "mediaFile.discTitleStreams": "Потоки тайтла",
  "mediaFile.discTitleStreamsUnknown":
    "Не удалось получить метаданные потоков этого тайтла.",
  "mediaFile.discTitleChapters": "Главы тайтла",
  "mediaFile.discTitleChaptersUnknown":
    "Не удалось получить метаданные глав этого тайтла.",
  "mediaFile.discChapterIdentity": "Глава {{id}}",
  "mediaFile.discEpisodeMapping": "Привязать тайтлы диска к эпизодам",
  "mediaFile.discMappingScope":
    "Выберите один эпизод для каждого тайтла воспроизведения, созданного на диске. Тайтлы без привязки не делают эпизоды доступными. ISO остаётся одним файлом.",
  "mediaFile.discMappingSave": "Сохранить привязки эпизодов",
  "mediaFile.discMappingFailed": "Не удалось сохранить привязки эпизодов",
  "mediaFile.discMappingLoading": "Загрузка эпизодов…",
  "mediaFile.discMappingNone": "Без эпизода",
  "mediaFile.discAutomatic": "Автоматически: самый длинный допустимый тайтл",
  "mediaFile.discSaving": "Проверка тайтла…",
  "mediaFile.discSelectionFailed": "Не удалось сохранить выбор тайтла диска",
  "mediaFile.analysisHeading": "Проверка медиа: {{status}}",
  "mediaFile.analysisLatestInspection":
    "Последняя проверка: {{status}}, {{time}}",
  "mediaFile.analysisPreviousRetained":
    "Предыдущая сохранённая проверка оставлена без изменений.",
  "mediaFile.analysisNoneSaved": "Для этого файла нет сохранённой проверки.",
  "mediaFile.analysisBudgetExhausted": "Лимит проверки исчерпан.",
  "mediaFile.analysisSummary":
    "Длительность: {{duration}} · общий: {{overall}} · видео: {{video}}",
  "mediaFile.analysisBudgetStopped":
    "Проверка остановлена по достижении лимита анализа.",
  "mediaFile.analysisProgram": "Программа {{id}}",
  "mediaFile.analysisProgramStreams": "потоки {{ids}}",
  "mediaFile.analysisSelected": "выбрано",
  "mediaFile.analysisChapters": "Главы",
  "mediaFile.analysisAttachments": "Вложения",
  "mediaFile.analysisUnknownType": "неизвестный тип",
  "mediaFile.analysisBytes": "{{count}} байт",
  "mediaFile.analysisCaptionService":
    "Служба субтитров {{standard}} № {{number}} · {{language}}",
  "mediaFile.analysisUnknownLanguage": "неизвестный язык",
  "mediaFile.streamUnknownCodec": "неизвестный кодек",
  "mediaFile.streamLanguage": "Язык",
  "mediaFile.streamProfileLevel": "Профиль / уровень",
  "mediaFile.streamBitrate": "Битрейт",
  "mediaFile.streamEstimatedBitrate": "Расчётный битрейт",
  "mediaFile.streamDimensions": "Размеры кадра",
  "mediaFile.streamPixelFormat": "Формат пикселей / глубина",
  "mediaFile.streamFieldOrder": "Порядок полей",
  "mediaFile.streamAspectRotation": "Соотношение сторон / поворот",
  "mediaFile.streamDeclaredFrameRate": "Заявленная частота кадров",
  "mediaFile.streamObservedFrameRate": "Наблюдаемая частота кадров",
  "mediaFile.streamVfrUnknown": "наличие переменной частоты кадров неизвестно",
  "mediaFile.streamVfrObserved": "обнаружена переменная частота кадров",
  "mediaFile.streamVfrNotObserved": "постоянная частота кадров",
  "mediaFile.streamHdr": "Признаки HDR",
  "mediaFile.streamNoHdr": "нет",
  "mediaFile.streamColor": "Основные цвета / передаточная функция / матрица",
  "mediaFile.streamContentLight": "MaxCLL / MaxFALL",
  "mediaFile.streamMasteringLuminance": "Яркость мастеринга",
  "mediaFile.streamDolbyVision": "Профиль / совместимость Dolby Vision",
  "mediaFile.streamChannels": "Каналы / схема расположения",
  "mediaFile.streamSampleRate": "Частота дискретизации / формат",
  "mediaFile.streamSampleDepth": "Разрядность",
  "mediaFile.streamUnknownLayout": "неизвестная схема расположения",
  "mediaFile.diagnose": "Проверить структурные заголовки",
  "mediaFile.diagnosing": "Проверка заголовков…",
  "mediaFile.diagnosticsFailed": "Не удалось проверить этот файл",
  "mediaFile.diagnosticsScope":
    "Выборочный анализ заголовков с ограниченным объёмом не проверяет каждый пакет, декодированное изображение или звуковое содержимое.",
  "mediaFile.diagnosticsRead":
    "Прочитано {{bytes}} байт за {{milliseconds}} мс",
  "mediaFile.diagnosticsSamples":
    "Проверено выборочно: транспортных пакетов — {{packets}}, заголовков кадров — {{frames}}",
  "mediaFile.diagnosticsCoverage":
    "Проверено {{percent}}% байтов исходного файла",
  "mediaFile.diagnosticsRanges": "Проверенные диапазоны байтов",
  "mediaFile.diagnosticsTruncated":
    "Некоторые диапазоны или предупреждения пропущены из-за достижения лимита отчёта.",
  "subtitle.subtitlesDisabledTitle": "Загрузка субтитров отключена",
  "subtitle.subtitlesDisabledBody":
    "Перед поиском субтитров включите их загрузку в настройках субтитров.",
  "settings.releaseNumberingLabel": "Нумерация релизов",
  "settings.releaseNumberingAuto": "Автоматически",
  "settings.releaseNumberingOfficial": "Официальный порядок",
  "settings.releaseNumberingAlternate": "Альтернативный порядок",
  "settings.releaseNumberingDvd": "Порядок DVD",
  "settings.minAvailability.default":
    "Использовать значение библиотеки по умолчанию",
  "status.blocklistEntryCleared": "Релиз убран из списка блокировки",
  "status.episodeFilesDeleteQueued":
    "Удаление файлов эпизодов ({{count}}) поставлено в очередь",
  "status.episodeFilesDeleted": "Удалено файлов эпизодов: {{count}}",
  "status.mediaFileDeleteQueued": "Удаление медиафайла поставлено в очередь",
  "status.mediaFileDeleted": "Медиафайл удалён",
  "status.renameQueued":
    "Переименование выполняется в фоновом режиме; ход выполнения отображается в разделе «Активность».",
  "status.searchResultsExpired":
    "Срок действия результатов поиска истёк, выполните поиск заново",
  "status.bulkRenamePreviewFailed":
    "Не удалось подготовить просмотр переименований для тайтлов: {{failed}}.",
  "status.bulkRenameFailed": "Не удалось переименовать выбранные тайтлы.",
  "status.bulkRenameQueued":
    "Тайтлы ({{count}}) переименовываются в фоновом режиме; ход выполнения отображается в разделе «Активность».",
  "status.bulkRenameForbidden":
    "У вас нет права переименовывать тайтлы во всех выбранных библиотеках.",
  "status.languageSaveFailed":
    "Не удалось сохранить язык в вашем профиле; в других браузерах останется текущий язык.",
  "status.catalogViewSaveFailed":
    "Не удалось сохранить этот макет в вашем профиле; он применяется только в этом браузере.",
  "status.userDeleted": "Пользователь {{name}} удалён.",
  "profile.passkeyDeleteConfirmTitle": "Удалить ключ доступа?",
  "profile.passkeyDeleteConfirmDescription":
    "С помощью этого ключа доступа больше нельзя будет войти в вашу учётную запись.",
  "wanted.searchJobProgress":
    "Выполнено шагов поиска: {{processed}}/{{total}}. {{title}}",
  "wanted.searchNoEligibleWork":
    "Нет отслеживаемых элементов, требующих загрузки или улучшения качества.",
  "wanted.searchNoAcceptableReleases":
    "Поиск завершён; подходящих релизов не найдено.",
  "wanted.searchSnapshotMissing":
    "Ответ поиска получен не полностью. Перед повторной попыткой проверьте раздел «Активность».",
  "wanted.searchInvalidSeason":
    "Не удалось определить этот сезон. Обновите тайтл и повторите попытку.",
  "wanted.decision.titleYearMismatch": "Другой год",
  "wanted.decision.episodicReleaseForMovie": "Эпизоды, а не фильм",
  "wanted.decision.minimumSeeders": "Слишком мало сидов",
  "wanted.decision.packBelowMissingThreshold":
    "Пакет не достигает порога недостающих эпизодов",
  "wanted.decision.packForSingleEpisode": "Пакет ради одного эпизода",
  "wanted.standby": "Резерв · {{count}}",
  "wanted.standbyTooltip":
    "Сохранённые кандидаты ({{count}}) будут опробованы, если текущая загрузка завершится неудачно.",
  "wanted.standbyScopeNote":
    "Считаются строки, привязанные к этой области; пакеты сезонов учитываются у опорного эпизода сезона.",
  "wanted.loadingStandby": "Загрузка сохранённых кандидатов…",
  "wanted.standbyCandidates": "Резервные кандидаты · {{count}}",
  "wanted.standbyRank": "Место {{rank}}",
  "wanted.standbyIndexer": "Индексатор",
  "wanted.standbySize": "Размер",
  "wanted.standbySeeders": "Сиды",
  "wanted.standbyScore": "Баллы",
  "wanted.standbyAge": "Возраст",
  "wanted.seasonPack": "Пакет сезона",
  "wanted.noStandbyCandidates": "нет резервных кандидатов",
  "setup.restoreRefusal.deadlineExceeded":
    "Восстановление превысило лимит в один час и было безопасно остановлено. Повторите попытку.",
  "setup.restoreRefusal.maintenanceActive":
    "Восстановление из копии или аварийное восстановление уже выполняется. Дождитесь завершения.",
  "setup.restoreRefusal.interrupted":
    "Восстановление прервано до завершения. Повторите попытку.",
  "setup.restoreRefusal.attemptNotFound":
    "Этой попытки восстановления больше не существует. Запустите восстановление заново.",
  "setup.restoreRefusal.recoveryRequired":
    "Не удалось подтвердить безопасность восстановления. Перед повторной попыткой проверьте журналы сервера.",
  "setup.restoreRefusal.operationInvalid":
    "Запросы восстановления должны отправляться отдельно. Перезагрузите страницу и повторите попытку.",
  "setup.restoreRefusal.restartFailed":
    "Восстановление подготовлено, но Scryer не удалось перезапуститься для его применения. Повторная попытка восстановления снова запустит перезапуск без повторной подготовки резервной копии; также можно перезапустить Scryer для завершения.",
  "setup.recommendedHeading": "Рекомендуется",
  "setup.recommendedHint":
    "Плагины, нужные большинству установок. Установите те, которые соответствуют вашему использованию Scryer.",
  "setup.recommendedArchiveTitle": "Распаковка архивов",
  "setup.recommendedArchiveReason":
    "Установите, если используете торренты или загружаете субтитры. И те и другие часто поставляются в архивах RAR или ZIP.",
  "setup.recommendedQbittorrentTitle": "qBittorrent",
  "setup.recommendedSubtitlesTitle": "Субтитры",
  "setup.recommendedSubtitlesReason":
    "Установите, если Scryer управляет вашими субтитрами. Enhanced Sync синхронизирует субтитры с видео, OpenSubtitles поддерживает фильмы и сериалы, а Jimaku и AnimeTosho — аниме.",
  "setup.recommendedAdvancedIndexersTitle":
    "Расширенная поддержка индексаторов",
  "setup.recommendedAdvancedIndexersReason":
    "Установите, если используете более сложные индексаторы Prowlarr, построенные на движке Cardigann.",
  "setup.recommendedMediaServerTitle": "Медиасервер",
  "setup.recommendedMediaServerReason":
    "Обновляет медиасервер после импорта в Scryer. Выберите тот, которым пользуетесь.",
  "setup.rulePacksHeading": "Наборы правил",
  "setup.rulePacksHint":
    "Правила оценки от сообщества, помогающие Scryer выбирать лучший релиз. Их можно изменить позже в разделе «Настройки → Правила».",
  "setup.rulePackTrashTitle": "TRaSH Guides",
  "setup.rulePackTrashReason":
    "Оценивает релизы по рекомендациям TRaSH Guides для релиз-групп, звука, HDR, размера и нежелательных релизов.",
  "setup.rulePackSeadexTitle": "SeaDex",
  "setup.rulePackSeadexReason":
    "Включите, если собираете аниме. Отдаёт предпочтение релизам, которые SeaDex считает лучшими из доступных.",
  "setup.rulePackUnavailable":
    "Сейчас недоступно. Обновите реестр, чтобы повторить попытку.",
  "setup.otherPluginsHeading": "Другие плагины",
  "setup.addMediaFolder": "Добавить папку",
  "setup.removeMediaFolder": "Убрать папку",
  "setup.mediaPathsNoneChosen":
    "Папки не выбраны, поэтому в этой библиотеке останутся прежние папки.",
  "setup.applyingImport": "Применение импорта…",
  "setup.applyingImportDetail":
    "Импортированное состояние отслеживания записывается в ваши библиотеки. Процесс продолжится, даже если вы покинете страницу.",
  "setup.importApplied": "Импорт применён",
  "setup.importAppliedDetail":
    "Состояние отслеживания каждого импортированного тайтла записано в ваши библиотеки.",
  "setup.importFinalizeFailedTitle": "Не удалось завершить импорт",
  "setup.titlesApplied": "Тайтлов: {{applied}} / {{total}}",
  "history.initial": "Первоначальный",
  "history.upgrade": "Улучшение качества",
  "history.prospectiveEpisodes": "Предполагаемые эпизоды",
  "history.eventTypes": "Типы событий",
  "history.eventTypesSelected": "Типов событий: {{count}}",
  "history.titleMoved": "Перемещён",
  "history.seedingStarted": "Сидирование",
  "history.seedingCompleted": "Сидирование завершено",
  "indexerErrors.title": "История ошибок индексаторов",
  "indexerErrors.scopedTitle": "{{indexer}} — история ошибок",
  "indexerErrors.history": "История ошибок",
  "indexerErrors.refresh": "Обновить",
  "indexerErrors.retry": "Повторить",
  "indexerErrors.loading": "Загрузка ошибок индексаторов…",
  "indexerErrors.loadingResponse": "Загрузка сохранённого ответа…",
  "indexerErrors.loadMore": "Загрузить ещё",
  "indexerErrors.noMore": "Больше нет сохранённых ответов",
  "indexerErrors.loadError":
    "Не удалось загрузить историю ошибок индексаторов.",
  "indexerErrors.detailLoadError": "Не удалось загрузить сохранённый ответ.",
  "indexerErrors.detailMissing": "Этот сохранённый ответ больше недоступен.",
  "indexerErrors.empty":
    "За последние 30 дней ошибки индексаторов не зафиксированы.",
  "indexerErrors.retentionNotice":
    "Ошибки индексаторов хранятся 30 дней. Для HTTP-ответов сохраняются заголовки и тело; при транспортных сбоях сохраняется только диагностика.",
  "indexerErrors.occurred": "Время возникновения",
  "indexerErrors.indexer": "Индексатор",
  "indexerErrors.operation": "Операция",
  "indexerErrors.http": "HTTP",
  "indexerErrors.classification": "Классификация",
  "indexerErrors.message": "Сообщение",
  "indexerErrors.contentType": "Тип содержимого",
  "indexerErrors.providerCode": "Код провайдера: {{code}}",
  "indexerErrors.expand": "Развернуть сохранённый ответ",
  "indexerErrors.collapse": "Свернуть сохранённый ответ",
  "indexerErrors.response": "Сохранённый HTTP-ответ",
  "indexerErrors.noHttpResponse":
    "HTTP-ответ не получен; для этого транспортного сбоя доступны только диагностические сведения.",
  "indexerErrors.headers": "Заголовки ответа",
  "indexerErrors.noHeaders": "Заголовки ответа не сохранены.",
  "indexerErrors.body": "Тело ответа",
  "indexerErrors.formatted": "С форматированием",
  "indexerErrors.raw": "Исходный вид",
  "indexerErrors.redacted": "[удалено перед сохранением]",
  "indexerErrors.base64Value": "Base64",
  "indexerErrors.previewTruncated":
    "Предпросмотр ограничен первым 1 MiB из {{size}}. Скачайте полное тело ответа, чтобы изучить все сохранённые байты.",
  "indexerErrors.emptyBody": "Тело ответа пустое.",
  "indexerErrors.binaryBody":
    "Этот ответ размером {{size}} не является корректным UTF-8. Скачайте его, чтобы изучить сохранённые байты.",
  "indexerErrors.downloadFullBody": "Скачать полное тело ответа",
  "settings.autoBackupsNextRunNeedsKey": "Отключено (сначала сохраните пароль)",
  "settings.autoBackupsKeyCleared": "Ключ резервного копирования удалён.",
  "settings.autoBackupsClearKeyTitle": "Удалить ключ резервного копирования?",
  "settings.autoBackupsClearKeyConfirm":
    "Сохранённый ключ автоматического резервного копирования будет удалён. Для восстановления существующих резервных копий, созданных с его помощью, он по-прежнему потребуется.",
  "settings.autoBackupsClearKeyConfirmBackupsOff":
    "Сохранённый ключ автоматического резервного копирования будет удалён, а автоматическое резервное копирование отключится до сохранения нового ключа. Для восстановления существующих резервных копий, созданных с его помощью, он по-прежнему потребуется.",
  "settings.autoBackupsKeyClearedBackupsOff":
    "Ключ резервного копирования удалён. Автоматическое резервное копирование отключено до сохранения нового ключа.",
  "settings.backupLocationResetTitle":
    "Использовать папку резервных копий по умолчанию?",
  "settings.backupLocationResetConfirm":
    "Новые резервные копии будут сохраняться в {{path}}. Существующие копии останутся на своих местах и будут скрыты, пока вы не вернётесь к прежней папке.",
  "settings.backupsDeletedCount": "Удалено резервных копий: {{count}}.",
  "settings.backupsDeleteFailedCount":
    "Не удалось удалить резервные копии: {{count}}.",
  "settings.backupsDeleteSelected": "Удалить выбранные",
  "settings.backupsDeleteSelectedConfirm":
    "Удалить с сервера выбранные наборы резервных копий ({{count}})? Скачанные копии не будут затронуты.",
  "settings.backupsSelectedCount": "Выбрано: {{count}}",
  "settings.backupsSelect": "Выбрать {{name}}",
  "settings.backupsSelectAll": "Выбрать все резервные копии",
  "settings.recycleBinPath": "Папка корзины",
  "settings.recycleBinPathPlaceholder":
    ".scryer-recycle в каждой корневой папке медиатеки",
  "settings.recycleBinPathHelp":
    "Оставьте поле пустым, чтобы использовать папку .scryer-recycle внутри каждой корневой папки медиатеки. Для другой папки необходимо указать абсолютный путь вне всех корневых папок медиатеки.",
  "settings.recycleBinPathFilesystemWarning":
    "Разместите папку в той же файловой системе, что и медиафайлы. Если файловые системы различаются, при перемещении в корзину каждый файл сначала копируется, а затем исходный файл удаляется. Это медленнее и разрывает жёсткие ссылки, общие с клиентом загрузки.",
  "settings.recycleBinRetentionDays": "Хранить файлы в корзине (дней)",
  "settings.recycleBinRetentionDaysHelp":
    "По истечении этого количества дней файлы из корзины удаляются безвозвратно.",
  "settings.recycleBinRetentionDaysInvalid":
    "Введите целое количество дней от {{min}} до {{max}}.",
  "settings.recycleBinEffectivePaths": "Сейчас удаляемые файлы попадают в:",
  "settings.recycleBinNoEffectivePaths":
    "Корневые папки медиатеки ещё не настроены.",
  "settings.recycleBinValidationError":
    "Автоматическая очистка приостановлена до устранения проблемы: {{error}}",
  "settings.recycleBinScheduledDeletion": "Удаление по расписанию",
  "settings.maintenanceActionSequence": "Последовательность действий",
  "settings.maintenanceSequenceTitle": "Последовательность действий",
  "settings.maintenanceSequenceHelp":
    "Выполнять эти шаги по порядку после окончания периода отсрочки для объекта. Пустая последовательность только фиксирует совпадения, не выполняя никаких действий.",
  "settings.maintenanceSequenceEmpty":
    "Только пробный запуск — шаги действий не настроены.",
  "settings.maintenanceSequenceObserveOnly": "Только пробный запуск",
  "settings.maintenanceSequenceSummary": "Сводка последовательности",
  "settings.maintenanceSequenceSummaryTarget": "Объекты: {{target}}",
  "settings.maintenanceSequenceSummaryArming": "При включении: {{arming}}",
  "settings.maintenanceSequenceSummaryEffects": "Последствия: {{effects}}",
  "settings.maintenanceSequenceTargetTitle": "соответствующий условию тайтл",
  "settings.maintenanceSequenceTargetTitleDescendants":
    "соответствующий условию тайтл, его сезоны и эпизоды",
  "settings.maintenanceSequenceTargetSeason": "соответствующий условию сезон",
  "settings.maintenanceSequenceTargetSeasonDescendants":
    "соответствующий условию сезон и его эпизоды",
  "settings.maintenanceSequenceTargetEpisode": "соответствующий условию эпизод",
  "settings.maintenanceSequencePreviewPlan":
    "Последствия по порядку для этого предпросмотра",
  "settings.maintenanceSequencePreviewDescendants": "включая дочерние объекты",
  "settings.maintenanceSequencePreviewMatchedScope":
    "только объекты выбранной области, соответствующие условию",
  "settings.maintenanceSequenceAdd": "Добавить шаг",
  "settings.maintenanceSequencePreset": "Шаблон последовательности",
  "settings.maintenanceSequencePresetApply": "Заменить шаблоном",
  "settings.maintenanceSequencePresetProfileThenSearch":
    "Изменить профиль, затем выполнить поиск, если он изменился",
  "settings.maintenanceSequencePresetUnmonitorThenDelete":
    "Отключить отслеживание дочерних объектов, затем удалить файлы",
  "settings.maintenanceSequenceTerminal": "Завершающий",
  "settings.maintenanceSequenceDependencies": "Требуется: {{steps}}",
  "settings.maintenanceSequenceIncludeDescendants": "Включить сезоны и эпизоды",
  "settings.maintenanceSequenceSearchCondition": "Условие поиска",
  "settings.maintenanceSequenceSearchUnconditional": "Всегда запрашивать поиск",
  "settings.maintenanceSequenceSearchPreviousProfileChanged":
    "Только после изменения профиля",
  "settings.maintenanceSequenceSearchRequested":
    "Этот шаг запрашивает поиск и отмечается как принятый только после принятия запроса. Дальнейшие действия по получению и загрузке могут оставаться приостановленными.",
  "settings.maintenanceSequenceHistoryWaiting": "ожидание предыдущего шага",
  "settings.maintenanceSequenceHistorySearchRequested": "поиск запрошен",
  "settings.maintenanceSequenceHistorySearchAccepted": "принято",
  "settings.maintenanceSequenceHistoryJob": "задание {{id}}",
  "settings.maintenanceSequenceCompletionAccepted": "Завершается при принятии",
  "settings.maintenanceSequenceCompletionCompleted":
    "Завершается по окончании выполнения",
  "settings.maintenanceSequenceStepUnavailable":
    "Этот сохранённый шаг недоступен в текущем каталоге. Удалите его или восстановите совместимый каталог перед сохранением или предпросмотром.",
  "settings.maintenanceSequenceCatalogUnavailable":
    "Каталог шагов действий недоступен. Сохранённые шаги по-прежнему видны, но сохранение и предпросмотр несохранённых изменений приостановлены до его загрузки.",
  "settings.maintenanceSequenceStepUnmonitor": "Отключить отслеживание",
  "settings.maintenanceSequenceStepDeleteFiles": "Удалить файлы",
  "settings.maintenanceSequenceStepChangeQualityProfile":
    "Изменить профиль качества",
  "settings.maintenanceSequenceStepSearch": "Поиск",
  "settings.maintenanceSequenceStepAddTags": "Добавить теги",
  "settings.maintenanceSequenceStepRemoveTags": "Удалить теги",
  "settings.maintenanceSequenceStepDeleteTitleAndFiles":
    "Удалить тайтл и файлы",
  "settings.maintenanceRuleStorageRoot": "Корневая папка хранилища",
  "settings.maintenanceRuleStorageRootNone":
    "Корневая папка хранилища не выбрана",
  "settings.maintenanceRuleStorageRootHelp":
    "Выберите настроенную корневую папку, чтобы получать сведения о её ёмкости и удалять только файлы, действительно расположенные в ней.",
  "settings.maintenanceRuleStorageRootUnmonitorHelp":
    "Выбранная корневая папка ограничивает только удаление файлов. Отключение отслеживания по-прежнему применяется ко всему соответствующему условию объекту, поэтому файлы в других корневых папках сохраняются, но больше не отслеживаются.",
  "settings.maintenanceRuleStorageRootClear":
    "Сбросить корневую папку хранилища",
  "settings.maintenanceStorageRootFileSummary":
    "Выбранная корневая папка: файлов — {{count}} · {{size}}",
  "settings.maintenanceRuleSaveDisabledNote":
    "Новые правила создаются отключёнными. Проверьте предпросмотр, попробуйте пробный запуск, затем включите правило.",
  "settings.maintenanceRuleState": "Состояние",
  "settings.maintenanceRuleStateHelp":
    "Отключённые правила никогда не проверяются. Пробный запуск фиксирует кандидатов без выполнения действий. Включённые правила выполняют действия над кандидатами по окончании периода отсрочки для каждого из них.",
  "settings.maintenanceRuleStateUpdated": "Правило «{{name}}» обновлено.",
  "settings.maintenanceRuleStateDisabled": "Отключено",
  "settings.maintenanceRuleStateDisabledHelp": "Никогда не проверяется.",
  "settings.maintenanceRuleStateDryRun": "Пробный запуск",
  "settings.maintenanceRuleStateDryRunHelp":
    "Фиксирует кандидатов; не выполняет действий.",
  "settings.maintenanceRuleStateEnabled": "Включено",
  "settings.maintenanceRuleStateEnabledHelp":
    "Выполняет действия над кандидатами по окончании их периода отсрочки.",
  "settings.maintenanceRuleEditedSinceEnabled":
    "Правило изменено после включения. Проверьте кандидатов и включите его снова.",
  "settings.maintenanceRuleSystemOwned": "Встроенное",
  "settings.expiredRequestLeasesHelp":
    "Удаляет запрошенный тайтл и его файлы после истечения срока хранения по запросу и окончания периода отсрочки. При изменении периода отсрочки правило переходит в режим пробного запуска, пока вы не включите его снова. Как и любое правило обслуживания, оно работает только при включённом обслуживании.",
  "settings.maintenanceSwitchEnabled": "Обслуживание",
  "settings.maintenanceSwitchEnabledHelp":
    "Проверять правила и показывать их кандидатов на этом экземпляре.",
  "settings.maintenanceSwitchActionsPaused": "Приостановить все действия",
  "settings.maintenanceSwitchActionsPausedHelp":
    "Во время приостановки включённые правила продолжают фиксировать кандидатов, но отслеживание не отключается, ничего не изменяется и не удаляется.",
  "settings.maintenanceSwitchesLocked":
    "Для просмотра переключателей обслуживания требуется разрешение на доступ к системным настройкам. Уточните у администратора, включено ли обслуживание и приостановлены ли действия.",
  "settings.maintenanceSwitchesUpdated": "Настройки обслуживания обновлены.",
  "settings.maintenanceUnpauseConfirmTitle":
    "Возобновить действия обслуживания?",
  "settings.maintenanceUnpauseConfirmBody":
    "Включённые правила будут выполнять действия над кандидатами по окончании периода отсрочки для каждого из них, в том числе правила, удаляющие файлы.",
  "settings.maintenanceUnpauseConfirmAction": "Возобновить действия",
  "settings.maintenanceCandidatesEmptyMaintenanceOff":
    "Обслуживание на этом экземпляре не включено, поэтому кандидаты не отображаются.",
  "settings.maintenanceCandidatesEmptyAllDisabled":
    "Все правила отключены, поэтому проверка не выполняется. Сначала переведите правило в режим пробного запуска.",
  "settings.maintenanceArmDestructiveIntro":
    "Правило «{{name}}» удаляет файлы. После включения оно будет выполнять действия над указанными ниже кандидатами по окончании периода отсрочки для каждого из них, если действия не приостановлены.",
  "settings.maintenanceArmDestructiveTitle":
    "Включить правило, удаляющее файлы?",
  "settings.maintenanceArmDestructiveConfirm": "Включить",
  "settings.maintenanceCandidatesTitle": "Скоро будут удалены",
  "settings.maintenanceCandidatesSubtitle":
    "Тайтлы, соответствующие правилу, начиная с ближайшего срока. Действия ни над одним из них ещё не выполнялись.",
  "settings.maintenanceFileSummary": "Файлов: {{count}} · {{size}}",
  "settings.maintenanceTestTitle": "Выберите тайтл для проверки",
  "settings.maintenanceNoSubjects":
    "В этом тайтле нет объектов для выбранной области действия.",
  "settings.maintenancePreviewExcluded": "Исключено из действий",
  "settings.maintenancePreviewEstimatedDue": "При совпадении сейчас: {{time}}",
  "settings.maintenancePreviewDue": "Окончание отсрочки: {{time}}",
  "settings.maintenancePreviewSubject": "Проверяемый объект",
  "settings.maintenancePreviewSample": "Выборка из медиатеки",
  "settings.maintenanceRuleScope": "Область действия правила",
  "settings.maintenanceRuleTemplate": "Шаблон",
  "settings.maintenanceRuleTemplatePlaceholder": "Начать с шаблона",
  "settings.maintenanceScopeSeason": "Сезон",
  "settings.maintenanceScopeEpisode": "Эпизод",
  "settings.maintenanceFileDeleted": "Удалён",
  "settings.maintenanceFilePending": "Ожидает",
  "settings.maintenanceFileResults": "Результаты обработки файлов",
  "settings.maintenanceFileProgress":
    "Удалено: {{completed}} · осталось: {{remaining}}",
  "settings.refMaintSubjectId":
    "ID тайтла, коллекции или эпизода Scryer для выбранной области действия",
  "settings.refMaintCollectionId":
    "ID коллекции сезона, которой принадлежит сезон или эпизод, если известен",
  "settings.refMaintFactsEpisodePositionByAirDate":
    "Позиция эпизода по дате выхода в рамках сериала",
  "settings.refMaintFactsSeasonPositionByAirDate":
    "Позиция сезона по самому новому подходящему загруженному эпизоду",
  "settings.refMaintFactsStorageRootId":
    "Настроенная корневая папка, выбранная для определения ёмкости хранилища и удаления файлов",
  "settings.refMaintFactsStorageAvailableBytes":
    "Количество байтов, доступных сейчас в выбранной корневой папке",
  "settings.refMaintFactsStorageTotalBytes":
    "Общая ёмкость выбранной корневой папки в байтах",
  "settings.refMaintFactsStorageAvailablePercent":
    "Процент доступного сейчас места в выбранной корневой папке",
  "settings.refMaintFactsListsEnabledIds":
    "Идентификаторы включённых списков, общедоступных или личных, в которых объект ещё присутствует",
  "settings.refMaintFactsListsPublicCount":
    "Количество общедоступных списков, в которых объект ещё присутствует",
  "settings.refMaintFactsListsPersonalCount":
    "Количество личных списков, в которых объект ещё присутствует",
  "settings.refMaintFactsListsFirstSeenAt":
    "Когда объект впервые появился в списке, в котором он всё ещё присутствует",
  "settings.refMaintFactsListsLastSeenAt":
    "Когда объект последний раз отмечался в списке, в котором он всё ещё присутствует",
  "settings.refMaintFactsListsDaysOnList":
    "Количество полных дней с первого появления объекта в списке, в котором он всё ещё присутствует",
  "settings.refMaintFactsListsBestRank":
    "Наилучшая (наименьшая) позиция объекта среди списков, в которых он всё ещё присутствует",
  "settings.refMaintFactsListsOnLeave":
    "Что делают включённые списки, содержащие объект, при его исключении",
  "settings.refMaintFactsListsLeftHandled":
    "Применено ли действие при исключении для каждого случая исключения из списка",
  "settings.refMaintFactsListsMembershipStates":
    "Состояния объекта в списках, в которых он всё ещё присутствует",
  "settings.refMaintFactsRequestOrigin":
    "Источник запроса, по которому был создан тайтл",
  "settings.refMaintFactsRequestListName":
    "Общедоступный список, из которого был запрошен тайтл",
  "settings.refMaintFactsEpisodePositionByAirDateValue":
    "Позиция 1 — самый новый подходящий загруженный эпизод, уже вышедший в эфир и не являющийся специальным. При одинаковых датах выхода порядок определяется номерами сезона и эпизода, затем стабильными ID.",
  "settings.refMaintFactsSeasonPositionByAirDateValue":
    "Позиция 1 — сезон с самым новым подходящим загруженным эпизодом, уже вышедшим в эфир и не являющимся специальным. Защищённые файлы также занимают позиции.",
  "settings.refMaintFactsStorageRootIdValue":
    "Постоянный идентификатор настроенной корневой папки медиатеки, выбранной в этой редакции",
  "settings.refMaintFactsStorageAvailableBytesValue":
    "Доступная ёмкость в байтах, измеренная непосредственно в выбранной корневой папке",
  "settings.refMaintFactsStorageTotalBytesValue":
    "Общая ёмкость в байтах, измеренная непосредственно в выбранной корневой папке",
  "settings.refMaintFactsStorageAvailablePercentValue":
    "Доступная ёмкость, делённая на общую ёмкость, от 0 до 100",
  "settings.refMaintFactsListsEnabledIdsValue":
    "Отсортированные идентификаторы списков; никогда не название списка",
  "settings.refMaintFactsListsPublicCountValue":
    "Количество общедоступных списков, включённых или приостановленных",
  "settings.refMaintFactsListsPersonalCountValue":
    "Количество личных списков, включённых или приостановленных",
  "settings.refMaintFactsListsFirstSeenAtValue":
    "Метка времени RFC3339; отсутствует с причиной not_on_a_list, если объект не содержится ни в одном списке",
  "settings.refMaintFactsListsLastSeenAtValue":
    "Метка времени RFC3339; отсутствует с причиной not_on_a_list, если объект не содержится ни в одном списке",
  "settings.refMaintFactsListsDaysOnListValue":
    "Полные дни до момента проверки; отсутствует с причиной not_on_a_list, если объект не содержится ни в одном списке",
  "settings.refMaintFactsListsBestRankValue":
    "Номер позиции; отсутствует с причиной no_list_rank, если ни один список, содержащий объект, не присваивает ему позицию",
  "settings.refMaintFactsListsOnLeaveValue":
    "Отсортированные значения из keep, log, unmonitor и tag",
  "settings.refMaintFactsListsLeftHandledValue":
    "True после обработки всех случаев исключения; отсутствует с причиной no_list_left, если объект не был исключён ни из одного списка",
  "settings.refMaintFactsListsMembershipStatesValue":
    "Отсортированные состояния, например in_library, added, requested или held",
  "settings.refMaintFactsRequestOriginValue":
    "Одно из значений manual, public_list или personal_list; отсутствует с причиной title_not_requested, если тайтл создан не по запросу",
  "settings.refMaintFactsRequestListNameValue":
    "Название этого общедоступного списка; отсутствует с причиной no_public_request_list для ручных запросов и запросов из личных списков, поскольку название личного списка никогда не раскрывается",
  "settings.refMaintFileIs3d":
    "True, если метаданные медиафайла указывают на 3D-видео. False означает, что оно не обнаружено, в том числе для ещё не проанализированных файлов.",
  "settings.requestRulesDefault":
    "Если нет включённых правил, запросы участников с разрешением на автоодобрение одобряются, а все остальные ожидают проверки.",
  "settings.requestRuleEnabledUpdated": "Правило «{{name}}» обновлено.",
  "settings.requestRuleRequesterFieldsHint":
    "Это правило использует поля автора запроса; для сохранения требуется разрешение на управление.",
  "settings.requestRuleSaveDisabledNote":
    "Новое правило сохраняется отключённым. Установите флажок «Включено» в таблице, чтобы оно могло принимать решения по запросам.",
  "settings.requestDecisionsSubtitle":
    "Недавние решения правил обработки запросов.",
  "requests.fallbackLibraryPermission":
    "Одобрено на основании разрешения автора запроса «Автоодобрение запросов».",
  "settings.refReqRequestList":
    "Список, отправивший запрос; см. input.request.list ниже. Отсутствует для ручного запроса",
  "settings.refReqSectionRequestList":
    "— Список, из которого поступил запрос, в input.request.list. Отсутствует для ручного запроса и если список не удалось прочитать",
  "settings.refReqRequestListSubscriptionId": "Идентификатор списка",
  "settings.refReqRequestListName":
    "Название списка; отсутствует для личного списка, название которого никогда не раскрывается",
  "settings.refReqRequestListScope": "public или personal",
  "settings.refReqRequestListMode":
    "Действие списка с его элементами: add, hold, request или discover",
  "settings.refReqRequestListProvider": "Провайдер списка",
  "settings.refReqRequestListOwnerIsRequester":
    "True, если владелец списка является автором запроса",
  "settings.refReqRequestListOwnerListPolicy":
    "Политика списков владельца: auto, approval или none; approval, если значение не сохранено",
  "settings.refReqRequestListRank":
    "Позиция элемента в списке; отсутствует, если список не присваивает ему позицию",
  "move.gatheringInfo": "Сбор всех сведений...",
  "move.methodManaged": "Scryer переместит файлы",
  "move.methodManual": "Я перемещу файлы сам",
  "move.manualLoading": "Загрузка инструкций по папкам...",
  "move.manualInstructions":
    "Переместите файлы, затем вернитесь и нажмите «Далее».",
  "move.changeDestination": "Изменить место назначения",
  "move.manualMappingChanged":
    "Расположение файлов в каталоге изменилось. Проверьте обновлённые папки, прежде чем снова нажать «Далее».",
  "move.manualStructure":
    "Используйте указанные ниже папки назначения, сохраняя имена файлов и относительную структуру папок. Кнопка «Далее» обновит каталог, исходя из того, что вы переместили файлы; Scryer не будет их проверять или изменять.",
  "move.manualSource": "Текущая папка",
  "move.manualDestination": "Переместить в папку",
  "move.retryInstructions": "Загрузить инструкции заново",
  "move.checkpointsShowAll": "Показать все тайтлы ({{count}})",
  "move.checkpointsShowLess": "Показать меньше тайтлов",
  "move.transferShowMore": "Показать ещё",
  "move.transferFilesForTitle": "Показать файлы тайтла «{{title}}»",
  "move.transferFilePath": "Файл",
  "move.transferCopyProgress": "Размещение",
  "move.transferPlacementAwaitingComparison": "Ожидание сравнения",
  "move.transferVerifyProgress": "Проверка",
  "move.transferVerificationBytes": "Проверено: {{done}} / {{total}}",
  "move.transferNoFiles": "Для этого тайтла нет перемещений файлов.",
  "move.transferFileState.QUEUED": "В очереди",
  "move.transferFileState.WAITING_FOR_STORAGE":
    "Ожидание хранилища · проверка каждые 2 мин",
  "move.transferFileState.DUPLICATE": "Дубликат (без копирования)",
  "move.transferFileState.MOVING": "Перемещение",
  "move.transferFileState.VERIFYING": "Проверка",
  "move.transferFileState.COMPARING": "Сравнение файлов",
  "move.transferFileState.DONE": "Готово",
  "move.transferFileState.FAILED": "Ошибка",
  "move.transferFileState.BLOCKED": "Заблокировано",
  "move.transferFileState.CANCELED": "Отменено",
  "move.transferFileState.NOT_PROCESSED": "Не обработано",
  "move.transferShowLess": "Показать меньше",
  "move.transferOverallProgress": "Общий прогресс",
  "move.transferEstimating": "Оценка…",
  "move.transferDisconnected": "Повторное подключение…",
  "move.transferEta": "Осталось около {{minutes}} мин",
  "move.transferColumnTitle": "Тайтл",
  "move.transferColumnStatus": "Статус",
  "move.transferColumnFiles": "Файлы: готово/всего",
  "move.transferColumnCurrentFile": "Текущий файл",
  "move.transferColumnProgress": "Прогресс",
  "move.transferRange": "{{from}}–{{to}} из {{total}}",
  "move.transferPrevious": "Назад",
  "move.transferNext": "Далее",
  "move.transferException": "Показать исключение для тайтла «{{title}}»",
  "move.transferActive":
    "Перемещается: {{copying}} · проверяется: {{verifying}}",
  "move.transferDetailPending": "Дополнительных сведений нет.",
  "move.retryAction": "Повторить",
  "move.retryRequested": "Повтор с последней проверенной контрольной точки.",
  "move.retryNotPossible": "Эту операцию нельзя повторить.",
  "move.retryFailed": "Не удалось повторить операцию.",
  "move.abandonAction": "Прекратить",
  "move.abandonRequested":
    "Операция прекращена. Её тайтлы и корневые папки освобождены; завершённые тайтлы не изменены. После восстановления хранилища кнопка «Повторить» переместит оставшиеся.",
  "move.abandonNotPossible": "Эта операция уже остановлена.",
  "move.abandonFailed": "Не удалось прекратить операцию.",
  "move.stalledNoRunner": "Нет исполнителя",
  "move.stalledGuidance":
    "Ни один исполнитель не работает над этой операцией. Если хранилище снова доступно, нажмите «Возобновить». Если оно не будет восстановлено, нажмите «Прекратить», чтобы освободить тайтлы и корневые папки; после этого будет доступен повтор.",
  "move.planAgainAction": "Спланировать заново",
  "move.planAgainTitleAction": "Спланировать этот тайтл заново",
  "move.planAgainFailed": "Не удалось открыть диалог перемещения.",
  "move.planAgainCatalogBlocked":
    "Тайтлы ({{count}}) исключены из повторного планирования, поскольку после проверки файлов обновление их записей в каталоге не завершилось; кнопка «Повторить» завершит его.",
  "move.reason.ownership_conflict":
    "Некоторые из этих тайтлов или корневых папок ещё заняты другой операцией. Дождитесь её завершения (или отмените её), затем повторите попытку.",
  "move.reason.storage_error":
    "Не удалось прочитать или записать файл. В источнике ничего не удалено. Устраните проблему с хранилищем (свободное место, права доступа, монтирование), затем повторите попытку; уже проверенные файлы не копируются повторно.",
  "move.reason.verification_mismatch":
    "Скопированный файл не совпал с исходным. Исходный файл не изменён, а копия не считается надёжной. Повторите попытку; если тот же файл снова не пройдёт проверку, проверьте диск назначения.",
  "move.reason.stale_plan":
    "После предпросмотра изменились данные, на которых основан план. Завершённые тайтлы находятся в согласованном состоянии. Спланируйте перемещение заново, чтобы продолжить с новым предпросмотром.",
  "move.reason.merge_unavailable":
    "Этот тайтл больше нельзя объединить с тайтлом в месте назначения. Исправьте тайтл назначения (или снимите с него выбор), затем спланируйте перемещение заново.",
  "move.reason.catalog_write_failed":
    "Файлы скопированы и проверены, но обновить каталог не удалось. В источнике ничего не удалено. Повторите попытку, чтобы завершить обновление каталога; не планируйте новое перемещение этого тайтла до успешного завершения.",
  "move.reason.source_cleanup_failed":
    "Перемещение завершено, но удалить старую копию не удалось. Повторите попытку, чтобы удалить её, либо убедитесь, что тайтл воспроизводится из нового места, и удалите старую папку самостоятельно.",
  "move.reason.stranded":
    "Операция была принята, но так и не запустилась. Повторите её или спланируйте перемещение заново.",
  "move.reason.canceled":
    "Операция отменена между тайтлами. Завершённые тайтлы не изменены; повторите попытку, чтобы переместить оставшиеся.",
  "move.reason.abandoned":
    "Операция прекращена. Завершённые тайтлы не изменены. Повторите попытку после восстановления хранилища.",
  "move.reason.root_unavailable":
    "Одна из необходимых этой операции корневых папок недоступна. Возобновите операцию после её повторного монтирования или прекратите операцию, чтобы освободить её тайтлы.",
  "move.transferDismiss": "Скрыть",
  "move.operationType.TITLE_FOLDER_RECOVERY":
    "Восстановление папки тайтла (оригиналы сохранены)",
  "indexerSearch.page.next": "Следующая страница",
  "indexerSearch.page.localSort":
    "Сортировка и фильтры применяются к этой странице. Каждая новая страница следует порядку сервера: сначала самые новые.",
  "indexerSearch.page.pending":
    "Некоторые индексаторы ещё выполняют поиск. Продолжайте проверять наличие новых результатов.",
  "indexerSearch.error.capacity":
    "Кэш поиска заполнен. Начните новый поиск после завершения других поисков.",
  "indexerSearch.error.cursor":
    "Индексатор повторил курсор страницы и не может продолжить этот поиск.",
  "indexerSearch.error.offset":
    "Недопустимые данные продолжения поиска. Начните поиск заново.",
  "indexerSearch.error.cancelled": "Этот поиск отменён.",
  "indexerSearch.error.expired":
    "Срок действия этой страницы истёк. Начните поиск заново.",
  "indexerSearch.error.limited":
    "Все выбранные индексаторы временно ограничили частоту запросов. Повторите попытку позже.",
  "indexerSearch.error.unavailable":
    "Ни один из выбранных индексаторов не смог ответить на этот поисковый запрос.",
  "indexerSearch.health.partial": "{{count}} · частично",
  "indexerSearch.caps.fresh": "Возможности из кэшированного снимка",
  "indexerSearch.caps.managed": "Возможности управляемого индексатора",
  "indexerSearch.caps.refreshed": "Возможности обновлены",
  "indexerSearch.caps.stale": "Используются прежние сведения о возможностях",
  "indexerSearch.caps.permissive":
    "Возможности неизвестны; выполняется попытка запрошенного поиска",
  "indexerSearch.caps.provider": "Возможности, заявленные провайдером",
  "indexerSearch.caps.unknown": "Источник сведений о возможностях недоступен",
  "indexerSearch.caps.refreshFailed":
    "Не удалось обновить сведения о возможностях",
  "indexerSearch.caps.refreshDeferred":
    "Обновление сведений о возможностях отложено",
  "indexerSearch.caps.refreshUnavailable":
    "Обновление сведений о возможностях недоступно",
  "indexerSearch.caps.retryAfter":
    "Задержка повтора, заданная провайдером: {{seconds}} с",
  "indexerSearch.facet.uploadFactor": "{{factor}}× отдача",
  "indexerSearch.row.swarm":
    "Сиды: {{seeders}} · личи: {{leechers}} · пиры: {{peers}}",
  "indexerSearch.detail.leechers": "Личи",
  "indexerSearch.detail.volumeFactors": "Коэффициент учёта загрузки / отдачи",
  "indexerSearch.detail.minimumSeedRatio":
    "Минимальный коэффициент сидирования",
  "indexerSearch.detail.minimumSeedTime": "Минимальное время сидирования",
  "grabDialog.assign.suggested": "Предлагаемые тайтлы",
  "grabDialog.routing.loading": "Определение клиентов загрузки…",
  "grabDialog.routing.single": "1 релиз",
  "grabDialog.routing.group": "Релизов: {{count}}",
  "grabDialog.category": "Категория",
  "grabDialog.category.default": "По умолчанию в клиенте",
  "grabDialog.footer.partial": "Отправлено: {{count}}",
  "grabDialog.cta.grab": "Загрузить",
  "title.tagsNoneDefined": "Теги ещё не созданы",
  "title.tagsCreateMore": "Создать ещё теги…",
  "title.catalogFilters.files": "Файлы",
  "title.catalogFilters.incompleteFiles": "Отсутствуют или частично доступны",
  "title.catalogFilters.onDiskFiles": "Полностью доступны или требуют внимания",
  "title.catalogFilters.presenceMissing": "Отсутствуют",
  "title.catalogFilters.presenceMissingHint": "На диске совсем нет файлов.",
  "title.catalogFilters.presencePartial": "Частично доступны",
  "title.catalogFilters.presencePartialHint":
    "Часть файлов есть на диске, но у некоторых отслеживаемых эпизодов, уже вышедших в эфир, файлов нет. Фильм хранится в одном файле, поэтому частичная доступность для него невозможна.",
  "title.catalogFilters.presenceComplete": "Полностью доступны",
  "title.catalogFilters.presenceCompleteHint":
    "Всё отслеживаемое есть на диске: у каждого отслеживаемого эпизода, уже вышедшего в эфир, есть файл либо присутствует файл самого фильма.",
  "title.catalogFilters.needsAttention": "Требует внимания",
  "title.catalogFilters.needsAttentionHint":
    "Есть файл, сканирование которого завершилось ошибкой или который требует проверки, поэтому перед использованием нужно разобраться с проблемой.",
  "mediaInfo.listingAgeDay": "{{count}} день",
  "lists.catalog.followed": "С подпиской",
  "lists.syncState.waiting": "Ожидание синхронизации",
  "lists.reason.mediaTypeNotIncluded":
    "Этот список не настроен на сохранение тайтлов такого типа.",
  "lists.reason.noRoute": "Для тайтлов такого типа не выбрана медиатека.",
  "lists.reason.duplicateTarget":
    "Другая запись в этом списке относится к тому же тайтлу.",
  "lists.reason.specials":
    "Специальные эпизоды из этого списка не сохраняются.",
  "lists.reason.filler": "Исключено настройкой филлеров.",
  "lists.reason.recap": "Исключено настройкой эпизодов-пересказов.",
  "lists.reason.rating": "Рейтинг ниже минимального.",
  "lists.reason.missingRating":
    "Нет рейтинга для сравнения с минимальным значением.",
  "lists.reason.genre": "Есть жанр или тема, которые этот список исключает.",
  "lists.reason.missingGenre":
    "Жанры пока неизвестны, поэтому фильтр жанров не может пропустить этот тайтл.",
  "lists.reason.adult":
    "Это тайтл для взрослых, а такие тайтлы исключены из этого списка.",
  "lists.reason.missingAdult":
    "Пока неизвестно, предназначен ли тайтл для взрослых, поэтому фильтр контента для взрослых не может его пропустить.",
  "lists.reason.releaseYear": "Год выхода за пределами диапазона.",
  "lists.reason.missingReleaseYear": "Год выхода неизвестен.",
  "lists.reason.format": "Формат не входит в число сохраняемых этим списком.",
  "lists.reason.language":
    "Язык оригинала не входит в число сохраняемых этим списком.",
  "lists.reason.missingLanguage": "Язык оригинала неизвестен.",
  "lists.reason.streamingService":
    "Доступно в одном из ваших стриминговых сервисов.",
  "lists.reason.unreleased": "Ещё не вышло.",
  "lists.reason.missingReleaseDate": "Дата выхода неизвестна.",
  "lists.reason.directorCredit": "Это не режиссёрская работа.",
  "lists.reason.sequelWithoutBase":
    "Это продолжение тайтла, которого нет в медиатеке.",
  "lists.reason.ambiguousSeriesMovie":
    "Запись может соответствовать нескольким фильмам серии, поэтому сопоставление не выполнено.",
  "lists.reason.notPermitted":
    "Владельцу списка не разрешено добавлять или запрашивать этот тайтл.",
  "lists.reason.refused": "В добавлении отказано.",
  "lists.reason.notFound": "При добавлении найти тайтл не удалось.",
  "lists.reason.actionFailed":
    "Не удалось добавить. При следующей синхронизации попытка повторится.",
  "lists.reason.requestRejected": "Запрос этого тайтла отклонён.",
  "lists.filter.ratings": "Рейтинги",
  "lists.filter.all": "Все",
  "lists.filter.any": "Хотя бы один",
  "lists.filter.match": "Соответствие минимальным рейтингам",
  "lists.filter.canonical": "Исключённые жанры/темы",
  "lists.filter.searchCanonical": "Поиск жанров и тем…",
  "lists.filter.vocabularyError":
    "Нормализованный справочник жанров и тем недоступен.",
  "lists.filter.retry": "Повторить",
  "lists.filter.unavailable": "Недоступно: {{label}}",
  "lists.filter.unresolved": "Требует исправления: {{label}}",
  "lists.filter.excludeAdult": "Исключить тайтлы для взрослых",
  "lists.filter.excludeAdultHelp":
    "Исключает тайтлы, отмеченные как предназначенные для взрослых. Тайтл с ещё неизвестными сведениями исключается до их получения.",
  "title.tagsCreateSelect": "Создать «{{label}}» и выбрать",
  "title.tagsSearch": "Найти или создать тег…",
  "lists.detail.olderRuns": "Показать более ранние синхронизации",
  "lists.refusal.alreadyFollowed": "На этот список уже есть подписка.",
  "lists.refusal.sourceRequired":
    "Выберите провайдера и список или вставьте ссылку.",
  "lists.refusal.linkNotRecognized":
    "Эта ссылка не ведёт на список, на который Scryer может подписаться.",
  "lists.refusal.providerNotInstalled": "Провайдер этого списка не установлен.",
  "lists.refusal.providerUnavailable":
    "Провайдер этого списка сейчас недоступен.",
  "lists.refusal.sourceNotOffered":
    "Этот провайдер не предоставляет такой список.",
  "lists.refusal.chartUnavailable": "Этот чарт больше недоступен.",
  "lists.refusal.memberAccountOnly":
    "Для этого списка нужна собственная учётная запись участника; подписаться на него для всех нельзя.",
  "lists.refusal.modeNotAllowed":
    "Это действие недоступно для списков такого типа.",
  "lists.refusal.kindsRequired": "Выберите хотя бы один тип тайтлов.",
  "lists.refusal.routeLibraryRequired":
    "Выберите медиатеку для каждого типа тайтлов.",
  "lists.refusal.routeLibraryNotFound":
    "Одна из выбранных медиатек больше не существует.",
  "lists.refusal.syncCapInvalid":
    "Максимальное количество тайтлов за синхронизацию должно быть не меньше 1.",
  "lists.refusal.disabled":
    "Этот список отключён. Включите его для синхронизации.",
  "lists.refusal.exclusionIdsRequired": "Для исключения нужен хотя бы один ID.",
  "lists.refusal.exclusionTitleRequired": "Для исключения нужен тайтл.",
  "lists.refusal.accountRequired": "Выберите подключённую учётную запись.",
  "lists.refusal.accountReconnectRequired":
    "Подключите эту учётную запись заново, чтобы продолжить её использование.",
  "lists.refusal.accountProviderMismatch":
    "Эта учётная запись относится к другому провайдеру.",
  "lists.refusal.accountIdentityMissing":
    "Провайдер не сообщил, кому принадлежит эта учётная запись. Подключитесь заново.",
  "lists.refusal.accountIdentityUnverified":
    "Не удалось подтвердить учётную запись. Подключите её заново.",
  "lists.refusal.accountLinkingUnsupported":
    "Этот провайдер не поддерживает подключение учётной записи.",
  "lists.refusal.accountLinkOriginInvalid":
    "Нельзя начать привязку учётной записи с этого адреса.",
  "lists.refusal.accountLinksTooMany":
    "Слишком много подключений учётных записей ожидают завершения. Завершите одно из них или дождитесь истечения его срока действия, затем подключитесь снова.",
  "lists.refusal.accountLinkResultInvalid":
    "Ответ провайдера непригоден для использования. Начните подключение заново.",
  "lists.refusal.accountLinkIssuerInvalid":
    "Ответ пришёл не от провайдера. Начните подключение заново.",
  "lists.refusal.accountKeyUnsupported":
    "Учётные записи этого провайдера подключаются не с помощью ключа.",
  "lists.refusal.accountKeyRequired":
    "Для подключения к этому провайдеру нужен ваш собственный ключ, но установленный плагин провайдера его не запрашивает. Администратору необходимо обновить плагин.",
  "lists.refusal.accountKeyInvalid":
    "Это не похоже на ключ. Вставьте его одной строкой без пробелов.",
  "lists.refusal.accountKeyRejected":
    "Провайдер не принял этот ключ. Убедитесь, что вы скопировали его целиком, и повторите попытку.",
  "lists.refusal.accountAuthUnavailable":
    "На этом сервере подключение учётных записей недоступно.",
  "lists.refusal.accountAuthAccessDenied":
    "Провайдер не предоставил доступ. Подключитесь заново и одобрите запрос.",
  "lists.refusal.accountAuthAuthorizationFailed":
    "Провайдер не принял подтверждение доступа. Начните подключение заново.",
  "lists.refusal.accountAuthBusy":
    "Сейчас Scryer обрабатывает слишком много подключений учётных записей. Повторите попытку чуть позже.",
  "lists.refusal.accountAuthInstanceUnrecognized":
    "Службе входа Scryer не удалось подтвердить этот сервер. Повторите попытку через несколько минут. Если проблема сохраняется, администратору следует проверить правильность часов сервера и доступ к интернету.",
  "lists.refusal.accountAuthInvalidProviderApp":
    "У собственного приложения этого сервера для провайдера указан адрес обратного вызова, не совпадающий с адресом, по которому открыт Scryer. Администратору необходимо исправить его в настройках провайдера.",
  "lists.refusal.accountAuthInvalidProviderResponse":
    "Ответ провайдера непригоден для использования. Начните подключение заново.",
  "lists.refusal.accountAuthInvalidRequest":
    "Запрос подключения непригоден для использования. Начните подключение заново.",
  "lists.refusal.accountAuthProviderNotConfigured":
    "Этот провайдер ещё не настроен для подключения учётных записей. Администратору необходимо завершить его настройку.",
  "lists.refusal.accountAuthProviderRejected":
    "Провайдер отказал в подключении. Начните подключение заново.",
  "lists.refusal.accountAuthProviderUnavailable":
    "Провайдер сейчас не отвечает. Повторите попытку позже.",
  "lists.refusal.accountAuthRateLimited":
    "Провайдер попросил Scryer снизить частоту запросов. Подождите минуту и повторите попытку.",
  "lists.refusal.accountAuthReconnectRequired":
    "Провайдер больше не принимает эти данные входа. Начните подключение заново.",
  "lists.refusal.accountAuthRelayUnavailable":
    "Служба входа Scryer сейчас не отвечает. Повторите попытку позже.",
  "lists.refusal.accountAuthTransportUnavailable":
    "Этому серверу не удалось связаться с провайдером. Проверьте его подключение к интернету и повторите попытку.",
  "lists.refusal.accountAuthUnsupported":
    "Учётные записи этого провайдера нельзя подключить таким способом.",
  "lists.refusal.accountAuthUnsupportedProviderApp":
    "Этот провайдер не поддерживает использование собственного приложения. Администратору необходимо удалить его из настроек провайдера.",
  "lists.refusal.providerAppUnsupported":
    "Этот провайдер не поддерживает использование собственного приложения.",
  "lists.refusal.providerAppRedirectInvalid":
    "URI перенаправления не является корректным URL.",
  "lists.refusal.providerAppIncomplete":
    "Для приложения провайдера нужны ID клиента, секрет, если провайдер его требует, и корректный URI обратного вызова для учётной записи.",
  "lists.exclusions.titlePlaceholder": "Найти тайтл",
  "lists.exclusions.bulkDeleteNoIds":
    "У некоторых удалённых тайтлов ({{count}}) нет внешних ID для исключения из списков.",
  "lists.exclusions.bulkDeleteFailed":
    "Не удалось исключить из списков некоторые удалённые тайтлы ({{count}}). Добавьте эти исключения на странице «Списки».",
  "lists.accounts.installProviders":
    "Установите и включите ниже провайдера личных списков, затем подключите здесь свою учётную запись.",
  "lists.plugins.heading": "Плагины провайдеров списков",
  "lists.accounts.askForProviders":
    "Попросите администратора установить и включить плагин провайдера списков, чтобы вы могли подключить свою учётную запись и подписываться на закрытые списки или списки для просмотра.",
  "profile.listAccounts": "Учётные записи личных списков",
  "profile.mediaServerAccounts": "Учётные записи медиасерверов",
  "lists.accounts.connectAnother": "Подключить другую учётную запись",
  "lists.accounts.connectAnotherHint":
    "Провайдер подключает ту учётную запись, в которую выполнен вход у него. Чтобы добавить другую, войдите в неё в окне провайдера.",
  "lists.accounts.notConnected": "Не подключена",
  "lists.accounts.needsReconnect": "Требуется повторное подключение",
  "lists.accounts.key.checking": "Проверка ключа у провайдера…",
  "lists.accounts.key.where": "Где его найти",
  "lists.accounts.key.private":
    "Ваш ключ хранится на этом сервере в зашифрованном виде, используется только для ваших личных списков и больше не отображается.",
  "lists.accounts.key.replace":
    "Сохранение нового ключа заменяет ключ, хранящийся для этой учётной записи.",
  "lists.accounts.key.approval":
    "После принятия ключа откроется окно провайдера, в котором нужно подтвердить подключение.",
  "lists.accounts.key.missing": "Сначала введите свой ключ.",
  "script.editor.jobNamePlaceholder": "Например, ночная очистка",
  "script.editor.inline": "Встроенный",
  "script.editor.interpreter": "Интерпретатор",
  "script.editor.interpreterHelp":
    "Интерпретатор для запуска скрипта. Укажите пути к интерпретаторам в разделе «Настройки > Скрипты».",
  "script.editor.format": "Форматировать отступы",
  "script.editor.formatHelp":
    "Переформатировать отступы скрипта с учётом выбранного языка (Shift+Alt+F).",
  "script.editor.assistanceHint":
    "Ctrl+Space: подсказки · Tab: отступ · Shift+Alt+F: форматирование отступов",
  "script.editor.injectedVariable": "Переменная окружения Scryer",
  "script.editor.snippet": "Фрагмент кода на выбранном языке",
  "script.editor.env.SCRYER_METADATA":
    "Метаданные JSON, описывающие этот импорт или задание по расписанию. Прочитайте их из окружения и разберите как JSON.",
  "script.editor.env.SCRYER_EVENT":
    "Событие, запустившее скрипт: post_import или scheduled_job.",
  "script.editor.env.SCRYER_FILE_PATH":
    "Полный путь к импортированному медиафайлу.",
  "script.editor.env.SCRYER_FACET":
    "Категория импортированного тайтла: movie, series или anime.",
  "script.editor.env.SCRYER_TITLE_NAME": "Название импортированного тайтла.",
  "script.editor.env.SCRYER_TITLE_ID":
    "Идентификатор импортированного тайтла в Scryer.",
  "script.editor.env.SCRYER_JOB_ID":
    "Идентификатор задания по расписанию в Scryer.",
  "script.editor.env.SCRYER_JOB_NAME": "Название задания по расписанию.",
  "script.editor.env.SCRYER_RUN_ID":
    "Идентификатор этого запуска задания по расписанию.",
  "script.editor.env.SCRYER_TRIGGER_SOURCE":
    "Способ запуска этого задания, например manual, startup или schedule.",
  "script.editor.inlineHelp": "Скрипт выполняется на выбранном выше языке.",
  "script.editor.filePathHelp": "Абсолютный путь к скрипту на хосте Scryer.",
  "script.editor.selectFile": "Выбрать файл скрипта",
  "script.editor.fileRequired": "Выберите файл скрипта перед сохранением.",
  "script.editor.required": "Необходимо указать название и скрипт.",
  "script.editor.schedule": "Расписание",
  "script.editor.blockingHelp":
    "Ожидает завершения скрипта и сохраняет его код выхода и вывод.",
  "script.editor.fireAndForgetHelp":
    "Запускает скрипт и фиксирует только факт запуска.",
  "script.language.shell": "Shell",
  "script.language.python": "Python",
  "script.language.powershell": "PowerShell",
  "script.language.batch": "Batch",
  "script.language.go": "Go",
  "script.output.filter": "Поток вывода",
  "script.output.filter.combined": "Объединённый",
  "script.output.filter.stdout": "Stdout",
  "script.output.filter.stderr": "Stderr",
  "script.output.stream.stdout": "stdout",
  "script.output.stream.stderr": "stderr",
  "script.schedule.kind": "Запуск",
  "script.schedule.kind.manual": "Вручную",
  "script.schedule.kind.interval": "С интервалом",
  "script.schedule.kind.daily": "Ежедневно",
  "script.schedule.kind.weekly": "Еженедельно",
  "script.schedule.kind.cron": "Cron",
  "script.schedule.runOnStartup": "Также запускать при старте Scryer",
  "script.schedule.manualHelp":
    "Задание выполняется только при нажатии «Запустить сейчас».",
  "script.schedule.every": "Каждые",
  "script.schedule.unit": "Единица",
  "script.schedule.unit.minutes": "Минуты",
  "script.schedule.unit.hours": "Часы",
  "script.schedule.unit.days": "Дни",
  "script.schedule.minimumInterval": "Минимальный интервал — одна минута.",
  "script.schedule.time": "Время",
  "script.schedule.days": "Дни",
  "script.schedule.day.monday": "Пн",
  "script.schedule.day.tuesday": "Вт",
  "script.schedule.day.wednesday": "Ср",
  "script.schedule.day.thursday": "Чт",
  "script.schedule.day.friday": "Пт",
  "script.schedule.day.saturday": "Сб",
  "script.schedule.day.sunday": "Вс",
  "script.schedule.cronExpression": "Выражение Cron",
  "script.schedule.cronHelp":
    "Пять полей: минута, час, день месяца, месяц, день недели.",
  "script.schedule.cronUnreadable": "Не удалось описать это выражение.",
  "script.schedule.invalid": "Некорректное расписание",
  "script.schedule.nextRuns": "Ближайшие запуски",
  "settings.scriptInterpreters.title": "Интерпретаторы скриптов",
  "settings.scriptInterpreters.description":
    "Программы, которые Scryer использует для запуска скриптов на каждом языке. Оставьте поле пустым, чтобы использовать программу, найденную в PATH.",
  "settings.scriptInterpreters.python": "Python",
  "settings.scriptInterpreters.powershell": "PowerShell",
  "settings.scriptInterpreters.batch": "Batch",
  "settings.scriptInterpreters.go": "Go",
  "settings.scriptInterpreters.containerHelp":
    "В образе контейнера Scryer нет интерпретаторов. Установите нужные и укажите здесь пути к ним.",
  "settings.scriptInterpreters.powershellHelp":
    "Скрипты PowerShell запускаются через pwsh — это PowerShell 7.",
  "settings.scriptInterpreters.saved": "Пути к интерпретаторам сохранены",
  "title.episodeCountOne.one": "{{count}} серия",
  "title.episodeCountOne.few": "{{count}} серии",
  "title.episodeCountOne.many": "{{count}} серий",
  "title.episodeCountOne.other": "{{count}} серии",
  "title.episodeCountOther.one": "{{count}} серия",
  "title.episodeCountOther.few": "{{count}} серии",
  "title.episodeCountOther.many": "{{count}} серий",
  "title.episodeCountOther.other": "{{count}} серии",
  "history.episodeCountMultiple.one": "{{count}} серия",
  "history.episodeCountMultiple.few": "{{count}} серии",
  "history.episodeCountMultiple.many": "{{count}} серий",
  "history.episodeCountMultiple.other": "{{count}} серии",
  "mediaInfo.listingAgeDay.one": "{{count}} день",
  "mediaInfo.listingAgeDay.few": "{{count}} дня",
  "mediaInfo.listingAgeDay.many": "{{count}} дней",
  "mediaInfo.listingAgeDay.other": "{{count}} дня",
  "mediaInfo.listingAgeDays.one": "{{count}} день",
  "mediaInfo.listingAgeDays.few": "{{count}} дня",
  "mediaInfo.listingAgeDays.many": "{{count}} дней",
  "mediaInfo.listingAgeDays.other": "{{count}} дня",
  "search.keepForDays.one": "{{count}} день",
  "search.keepForDays.few": "{{count}} дня",
  "search.keepForDays.many": "{{count}} дней",
  "search.keepForDays.other": "{{count}} дня",
  "dashboard.attentionCount.one": "{{count}} элемент требует внимания",
  "dashboard.attentionCount.few": "{{count}} элемента требуют внимания",
  "dashboard.attentionCount.many": "{{count}} элементов требуют внимания",
  "dashboard.attentionCount.other": "{{count}} элемента требуют внимания",
  "dashboard.pluginUpdateCount.one": "{{count}} обновление плагина",
  "dashboard.pluginUpdateCount.few": "{{count}} обновления плагинов",
  "dashboard.pluginUpdateCount.many": "{{count}} обновлений плагинов",
  "dashboard.pluginUpdateCount.other": "{{count}} обновления плагинов",
  "dashboard.rootCount.one": "{{count}} корневая папка",
  "dashboard.rootCount.few": "{{count}} корневые папки",
  "dashboard.rootCount.many": "{{count}} корневых папок",
  "dashboard.rootCount.other": "{{count}} корневой папки",
  "settings.recycleBinFileCountOne.one": "{{count}} файл",
  "settings.recycleBinFileCountOne.few": "{{count}} файла",
  "settings.recycleBinFileCountOne.many": "{{count}} файлов",
  "settings.recycleBinFileCountOne.other": "{{count}} файла",
  "settings.recycleBinFileCountOther.one": "{{count}} файл",
  "settings.recycleBinFileCountOther.few": "{{count}} файла",
  "settings.recycleBinFileCountOther.many": "{{count}} файлов",
  "settings.recycleBinFileCountOther.other": "{{count}} файла",
  "wanted.totalCount.one": "{{count}} элемент",
  "wanted.totalCount.few": "{{count}} элемента",
  "wanted.totalCount.many": "{{count}} элементов",
  "wanted.totalCount.other": "{{count}} элемента",
  "uiText.tables": "Таблицы",
  "uiText.table": "Таблица",
  "uiText.rows": "Строки",
  "uiText.restoreWarnings": "Предупреждения восстановления",
  "uiText.scoringPreviewOnlyThisDoesNotSave":
    "Только предварительный просмотр оценки. Он не сохраняет",
  "uiText.admitADownloadOrCreateHistory":
    ", не разрешает загрузку и не создаёт записи в истории.",
  "uiText.libraryTitle": "Тайтл библиотеки",
  "uiText.sizeGiBOptional": "Размер (ГиБ, необязательно)",
  "uiText.inputsChangedResultIsStale":
    "Входные данные изменились; результат устарел.",
  "uiText.incompleteScoringPreview": "Неполный предварительный просмотр оценки",
  "uiText.thisResultIncludesEvaluationErrorsSoItCannotDetermineDownloadAdmission":
    "Результат содержит ошибки оценки, поэтому по нему нельзя определить, разрешена ли загрузка.",
  "uiText.profile": "Профиль:",
  "uiText.context": "Контекст:",
  "uiText.tags": "Теги:",
  "uiText.evaluationErrors": "Ошибки оценки",
  "uiText.createAnIntegrationAPIKeyInWeaver":
    "Создайте ключ API интеграции в Weaver:",
  "uiText.openWeaverSecuritySettings": "откройте настройки безопасности Weaver",
  "uiText.finishTheWeaverURLAboveToGenerateTheLink":
    "заполните URL Weaver выше, чтобы создать ссылку.",
  "uiText.discoveringProwlarrIndexers": "Обнаружение индексаторов Prowlarr…",
  "uiText.sponsor": "Поддержать проект",
  "uiText.social": "Сообщество",
  "uiText.bit": " бит",
  "uiText.kbps": "Кбит/с",
  "uiText.forced": "[Принудительно]",
  "uiText.default": "[По умолчанию]",
  "uiText.matchFilesToEpisodesOrSeriesMoviesFor":
    "Сопоставьте файлы с сериями или фильмами сериала для",
  "uiText.openDownloadClientsSettings": "Открыть настройки клиентов загрузки",
  "uiText.thisDownloadContainsArchivesExtractThemBeforeMappingMediaFiles":
    "Эта загрузка содержит архивы. Распакуйте их перед сопоставлением медиафайлов.",
  "uiText.extractArchivesPreview": "Распаковать архивы и просмотреть",
  "uiText.theVideoFilesInThisDownloadDoNotHaveARecognizedExtension":
    "У видеофайлов в этой загрузке нет распознаваемого расширения.",
  "uiText.queueing": "Добавление в очередь…",
  "uiText.more": "ещё",
  "uiText.noSubs": "Нет субтитров",
  "uiText.mON": "МОНИТ.",
  "uiText.oAuthApplications": "Приложения OAuth",
  "uiText.jellyfinPluginOAuth": "OAuth плагина Jellyfin",
  "uiText.createAStandaloneOAuthClientForTheJellyfinPluginThisDoesNotRequireAJellyfinMediaServerConnectionOrAccountLinking":
    "Создайте отдельный клиент OAuth для плагина Jellyfin. Подключение медиасервера Jellyfin или привязка аккаунтов для этого не требуются.",
  "uiText.oAuthClient": "Клиент OAuth:",
  "uiText.automaticAccountLinking": "Автоматическая привязка аккаунтов:",
  "uiText.accountLinkingCannotUseThisPublicURL":
    "Этот публичный URL нельзя использовать для привязки аккаунтов:",
  "uiText.thePluginStillSignsInItGrantsLibraryAccessOnlyUntilThisIsFixed":
    "Вход через плагин по-прежнему работает; до устранения проблемы он предоставляет только доступ к библиотеке.",
  "uiText.moreThanOneOAuthClientIsRegisteredAsTheJellyfinPluginDeleteAllButOneBeforeThePluginSignsIn":
    "Для плагина Jellyfin зарегистрировано несколько клиентов OAuth. Перед входом через плагин оставьте только один, удалив остальные.",
  "uiText.thisClientHasNoCallbackURLRegisteredEditItToAddThePluginCallback":
    "Для этого клиента не зарегистрирован URL обратного вызова. Добавьте адрес обратного вызова плагина в настройках клиента.",
  "uiText.publicJellyfinBaseURL": "Публичный базовый URL Jellyfin",
  "uiText.createJellyfinPluginClient": "Создать клиент плагина Jellyfin",
  "uiText.jellyfinPluginOAuthClientReconciliationPending":
    "Ожидается сверка клиента OAuth плагина Jellyfin",
  "uiText.retryClientList": "Повторить загрузку списка клиентов",
  "uiText.managedApplications": "Управляемые приложения",
  "uiText.managedByScryer": "Управляется Scryer",
  "uiText.customApplications": "Пользовательские приложения",
  "uiText.noCustomOAuthApplicationsAreRegistered":
    "Пользовательские приложения OAuth не зарегистрированы.",
  "uiText.displayName": "Отображаемое имя",
  "uiText.hTTPSCallbackURLs": "URL обратного вызова HTTPS",
  "uiText.oneExactHTTPSURLPerLine": "По одному точному URL HTTPS в строке.",
  "uiText.multiFactorAuthentication": "Многофакторная аутентификация",
  "uiText.local": "· локальный",
  "uiText.theSelectedEmbyConnectUserMustBeALocalAdministratorForSetup":
    "Для настройки выбранный пользователь Emby Connect должен быть локальным администратором.",
  "uiText.embyConnectUsernameOrEmail":
    "Имя пользователя или электронная почта Emby Connect",
  "uiText.embyServer": "Сервер Emby",
  "uiText.remote": "· удалённый",
  "uiText.allowUsersToSignInThroughEmbyConnect":
    "Разрешить вход через Emby Connect",
  "uiText.testConnect": "Проверить Connect",
  "uiText.defaultPermissions": "Разрешения по умолчанию",
  "uiText.allowed": "Разрешено",
  "uiText.denied": "Запрещено",
  "uiText.aPIKeys": "Ключи API",
  "uiText.copyThisKeyNow": "Скопируйте этот ключ сейчас",
  "uiText.forYourSecurityScryerWillNotDisplayItAgain":
    "В целях безопасности Scryer больше не покажет этот ключ.",
  "uiText.copyKey": "Скопировать ключ",
  "uiText.aPIKeyCreationIsDisabledByTheCurrentSecurityPolicy":
    "Создание ключей API запрещено текущей политикой безопасности.",
  "uiText.loadingAPIKeys": "Загрузка ключей API…",
  "uiText.noAPIKeysYet": "Ключей API пока нет.",
  "uiText.expires": "Истекает",
  "uiText.sponsorButton": "Кнопка поддержки проекта",
  "uiText.showTheSponsorLinkInTheNavigationFooter":
    "Показывать ссылку поддержки проекта внизу панели навигации.",
  "uiText.connectedApps": "Подключённые приложения",
  "uiText.oAuthIntegrationsAuthorizedToAccessScryerAsYou":
    "Интеграции OAuth, которым разрешён доступ к Scryer от вашего имени.",
  "uiText.noConnectedApps": "Нет подключённых приложений.",
  "uiText.authorized": "Доступ разрешён:",
  "uiText.lastUsed": "Последнее использование:",
  "uiText.revoke": "Отозвать доступ",
  "uiText.linkEmbyAccount": "Привязать аккаунт Emby",
  "uiText.verifyAccountSecurity": "Подтверждение безопасности аккаунта",
  "uiText.verifyACurrentFactorBeforeManagingPasskeysTOTPOrRecoveryCodes":
    "Перед управлением ключами доступа, TOTP или кодами восстановления подтвердите личность действующим фактором аутентификации.",
  "uiText.tOTPOrRecoveryCode": "TOTP или код восстановления",
  "uiText.verifyPassword": "Подтвердить паролем",
  "uiText.verifyWithPasskey": "Подтвердить ключом доступа",
  "uiText.verifyCode": "Проверить код",
  "uiText.thisAccountHasNoAvailableReauthenticationFactorSignOutAndSignInAgainThenRetryTheAccountSecurityChange":
    "У этого аккаунта нет доступного фактора повторной аутентификации. Выйдите и войдите снова, затем повторите изменение параметров безопасности аккаунта.",
  "uiText.community": "Сообщество",
  "uiText.backToPacks": "← К пакетам",
  "uiText.chooseTheRulesToEnableAfterInstallationRulesStayManaged":
    "Выберите правила для включения после установки. Правила остаются управляемыми",
  "uiText.managedByThePackAuthor": "Управляется автором пакета.",
  "uiText.existingCustomCopiesAreNotAdoptedOrDisabledWhenThisPackIsInstalled":
    "При установке этого пакета существующие пользовательские копии не передаются под его управление и не отключаются.",
  "uiText.toNotifyAMediaServerIeJellyfinFirstCreateA":
    "Чтобы отправлять уведомления медиасерверу (например, Jellyfin), сначала создайте",
  "uiText.mediaServerConnection": "подключение медиасервера",
  "uiText.enterDetailsManually": "Ввести сведения вручную",
  "uiText.bundle": "Архив",
  "uiText.filesystem": "Файловая система",
  "uiText.serviceHealth": "Состояние сервисов",
  "uiText.datastore": "Хранилище данных",
  "uiText.migration": "Миграция",
  "uiText.indexerStatsLast24h": "Статистика индексаторов за 24 часа",
  "uiText.countsCoverRequestsScryerSendsProwlarrChildCountsDoNotIncludeRequestsProwlarrMakesInternally":
    "Счётчики учитывают запросы, отправленные Scryer. Счётчики дочерних индексаторов Prowlarr не включают внутренние запросы Prowlarr.",
  "uiText.lastQuery": "Последний запрос:",
  "uiText.observedCallsToday": "Зафиксировано вызовов сегодня:",
  "uiText.since": "(с",
  "uiText.observedSince": "Наблюдение с",
  "uiText.searchRuns": "Запуски поиска:",
  "uiText.failed": "с ошибкой)",
  "uiText.providerAPIUsage": "Использование API провайдера:",
  "uiText.providerGrabUsage": "Использование загрузок провайдера:",
  "uiText.noIndexerActivityInTheLast24Hours":
    "За последние 24 часа активности индексаторов не было.",
  "uiText.eventsLoaded": "событий загружено",
  "uiText.movie": "фильм",
  "uiText.wantedItem": "Искомый элемент",
  "uiText.confirmItSYou": "Подтвердите свою личность",
  "uiText.completeAnEnrolledAuthenticationFactorToFinishSigningIn":
    "Для завершения входа используйте настроенный фактор аутентификации.",
  "uiText.useAnAuthenticatorOrRecoveryCodeInstead":
    "Использовать приложение-аутентификатор или код восстановления",
  "uiText.canTUseYourPasskeyContactAnAdministratorToRecoverYourAccount":
    "Не удаётся использовать ключ доступа? Обратитесь к администратору для восстановления аккаунта.",
  "uiText.backToSignIn": "Вернуться ко входу",
  "uiText.protectYourAccountWithAPasskeyOrAnAuthenticatorApp":
    "Защитите аккаунт ключом доступа или приложением-аутентификатором.",
  "uiText.createAPasskey": "Создать ключ доступа",
  "uiText.useAnAuthenticatorAppInstead":
    "Использовать приложение-аутентификатор",
  "uiText.keepMeSignedIn": "Не выходить из аккаунта",
  "uiText.signInWithEmby": "Войти через Emby",
  "uiText.completingAuthorization": "Завершение авторизации…",
  "uiText.authorize": "Разрешить доступ",
  "uiText.scopes": "Области доступа",
  "uiText.libraryAccess": "Доступ к библиотеке",
  "uiText.jellyfinAccountLinking": "Привязка аккаунта Jellyfin",
  "uiText.accountLinkingNotGrantedNoEligibleJellyfinConnection":
    "Привязка аккаунта не разрешена: нет подходящего подключения Jellyfin.",
  "uiText.retryValidation": "Повторить проверку",
  "uiText.deny": "Запретить",
  "uiText.isReadyToApply": "готово к применению.",
  "uiText.installedRulePacks": "Установленные пакеты правил",
  "uiText.packAuthoredNamesDescriptionsAndFacetsStayManagedSomePacksAllowIndividualRulesToBeCopiedForCustomization":
    "Названия, описания и категории, заданные автором пакета, остаются управляемыми. Некоторые пакеты позволяют копировать отдельные правила для настройки.",
  "uiText.pack": "Пакет",
  "uiText.lastUpdated": "Последнее обновление",
  "uiText.autoUpdate": "Автообновление",
  "uiText.available": "доступно",
  "uiText.updateFailed": "Обновление не удалось",
  "uiText.check": "Проверить",
  "uiText.removedUpstream": "Удалено из источника",
  "uiText.serviceIsRestarting": "Сервис перезапускается…",
  "uiText.noArt": "Нет изображения",
  "uiText.loadingView": "Загрузка раздела…",
  "uiText.loadingLibrary": "Загрузка библиотеки…",
  "uiText.checkingYourTitlesAndLibrarySetup":
    "Проверка тайтлов и настроек библиотеки.",
  "uiText.currentDownload": "Текущая загрузка:",
  "uiText.state": "Состояние:",
  "uiText.pageNotFound": "Страница не найдена",
  "uiText.thePageYouRequestedDoesNotExist":
    "Запрошенная страница не существует.",
  "uiText.goToHome": "На главную",
  "uiText.liveServiceOutput": "Вывод сервиса в реальном времени",
  "uiText.buffered": "в буфере",
  "uiText.line": "Строка",
  "uiText.shown": "показано ·",
  "uiText.matching": "соответствует фильтру",
  "uiText.noLogsAvailableYet": "Журналы пока недоступны.",
  "uiText.thisLegacyTextLogLineHasNoJSONEventPayload":
    "Эта строка старого текстового журнала не содержит события JSON.",
  "uiText.aPIExplorerIsUnavailable": "Обозреватель API недоступен.",
  "uiText.home": "Главная",
  "uiText.managedTRaSHGuides": "Управляемые TRaSH Guides",
  "uiText.scoreOnlyLocalePackFor": "языковой пакет только для оценки для",
  "routeError.notFoundDetail": "Страница не существует или была перемещена.",
  "routeError.requestFailed": "Запрос не выполнен",
  "routeError.serverStatus": "Сервер вернул ошибку ({{status}}).",
  "routeError.somethingWrong": "Что-то пошло не так",
  "routeError.unexpected": "Непредвиденная ошибка",
  "routeError.unknownDetail":
    "При загрузке этой страницы произошла неизвестная ошибка.",
  "dashboard.metricMovies.one": "фильм",
  "dashboard.metricMovies.few": "фильма",
  "dashboard.metricMovies.many": "фильмов",
  "dashboard.metricMovies.other": "фильма",
  "dashboard.metricSeries.one": "сериал",
  "dashboard.metricSeries.few": "сериала",
  "dashboard.metricSeries.many": "сериалов",
  "dashboard.metricSeries.other": "сериала",
  "auth.embyConnectMode": "Emby Connect",
  "rules.previewSavedSafety":
    "Только предварительная оценка. Она не сохраняет это правило, не разрешает загрузку и не создаёт историю.",
  "rules.previewDraftSafety":
    "Только предварительная оценка. Она не сохраняет этот черновик, не разрешает загрузку и не создаёт историю.",
  "rules.packChooseCustomizable":
    "Выберите правила, которые нужно включить после установки. Правила остаются управляемыми; каждое из них можно скопировать для собственной настройки.",
  "rules.packChooseManaged":
    "Выберите правила, которые нужно включить после установки. Правила остаются управляемыми.",
  "uiAttr.18_and_over": "Только для совершеннолетних",
  "uiAttr.My_Indexer": "Мой индексатор",
  "uiAttr.Select_provider": "Выберите провайдера",
  "uiAttr.My_Download_Client": "Мой клиент загрузки",
  "uiAttr.Social_links": "Сообщество",
  "uiAttr.Cancel_import": "Отменить импорт",
  "uiAttr.This_stops_the_queued_or_active_import_Source_files_are_preserved_and_only_temporary_import_output_is_removed":
    "Импорт в очереди или выполняющийся импорт будет остановлен. Исходные файлы сохранятся; удалены будут только временные результаты импорта.",
  "uiAttr.Calendar_monitoring_filter": "Фильтр отслеживания в календаре",
  "uiAttr.Calendar_media_types": "Типы контента в календаре",
  "uiAttr.Refresh_requests": "Обновить запросы",
  "uiAttr.Request_filters": "Фильтры запросов",
  "uiAttr.Level": "Уровень",
  "uiAttr.Filter_log_text": "Фильтр текста журнала…",
  "uiAttr.App": "Приложение",
  "uiAttr.Replace_in_progress_download": "Заменить выполняющуюся загрузку?",
  "uiAttr.Toggle_Sidebar": "Переключить боковую панель",
  "uiAttr.API_explorer": "Обозреватель API",
  "uiAttr.Search_movies_shows_and_anime": "Поиск фильмов, сериалов и аниме",
  "uiAttr.Library_title": "Материал в библиотеке",
  "uiAttr.Episode": "Эпизод",
  "uiAttr.Unknown": "Неизвестно",
  "uiAttr.New_API_key": "Новый API-ключ",
  "uiAttr.e_g_Home_automation": "Например, домашняя автоматизация",
  "uiAttr.Expires": "Срок действия",
  "uiAttr.Revoke_API_key": "Отозвать API-ключ?",
  "uiAttr.Copy_Jellyfin_plugin_callback_URL":
    "Скопировать URL обратного вызова плагина Jellyfin",
  "uiAttr.Example_integration": "Пример интеграции",
  "uiAttr.Enabled": "Включено",
  "uiAttr.Disabling_immediately_revokes_all_existing_grants_and_tokens_for_this_application":
    "При отключении немедленно отзываются все существующие разрешения и токены этого приложения.",
  "uiAttr.Delete_OAuth_application": "Удалить приложение OAuth?",
  "uiAttr.Deleting_this_application_immediately_revokes_its_existing_OAuth_grants_and_tokens_This_cannot_be_undone":
    "При удалении приложения немедленно отзываются все выданные ему разрешения и токены OAuth. Это действие нельзя отменить.",
  "uiAttr.Copy_OAuth_client_ID": "Скопировать ID клиента OAuth",
  "uiAttr.Current_password": "Текущий пароль",
  "uiAttr.TOTP_or_recovery_code": "TOTP или код восстановления",
  "uiAttr.Restrict_API_keys_to_system_settings_users":
    "Разрешать API-ключи только пользователям с доступом к системным настройкам",
  "uiAttr.Require_MFA_for_Emby_login": "Требовать MFA при входе через Emby",
  "uiAttr.Show_Sponsor_button": "Показывать кнопку поддержки проекта",
  "uiAttr.Permissions": "Разрешения",
  "uiAttr.Block_releases_over_100_GiB":
    "Блокировать релизы размером более 100 ГиБ",
  "uiAttr.My_Webhook": "Мой вебхук",
  "uiAttr.Installed_rule_packs": "Установленные наборы правил",
  "uiAttr.Apply_rule_pack_update": "Применить обновление набора правил",
  "uiAttr.Uninstall_rule_pack": "Удалить набор правил",
  "uiAttr.Select_a_server": "Выберите сервер",
  "uiAttr.Preparing_authorization": "Подготовка авторизации",
  "uiAttr.Granted_scopes": "Предоставленные права доступа",
  "uiAttr.Authenticator_or_recovery_code":
    "Код приложения-аутентификатора или код восстановления",
  "uiAttr.Enter_a_code_from_your_authenticator_app_or_a_recovery_code":
    "Введите код из приложения-аутентификатора или код восстановления.",
  "catalog.summary.movie": "{{formattedCount}} фильм",
  "catalog.summary.movies": "{{formattedCount}} фильмов",
  "catalog.summary.series": "{{formattedCount}} сериалов",
  "catalog.summary.anime": "{{formattedCount}} аниме",
  "catalog.summary.movies.one": "{{formattedCount}} фильм",
  "catalog.summary.movies.few": "{{formattedCount}} фильма",
  "catalog.summary.movies.many": "{{formattedCount}} фильмов",
  "catalog.summary.movies.other": "{{formattedCount}} фильма",
  "catalog.summary.series.one": "{{formattedCount}} сериал",
  "catalog.summary.series.few": "{{formattedCount}} сериала",
  "catalog.summary.series.many": "{{formattedCount}} сериалов",
  "catalog.summary.series.other": "{{formattedCount}} сериала",
  "requests.queueTitle": "Очередь запросов",
  "requests.myTitle": "Мои запросы",
  "requests.status.dismissed": "Отклонено",
  "label.movie": "Фильм",
  "label.all": "Все",
  "calendar.locale": "ru",
  "calendar.today": "Сегодня",
  "calendar.month": "Месяц",
  "calendar.week": "Неделя",
  "calendar.type.anime": "Аниме",
  "calendar.type.movie": "Фильм",
  "calendar.type.series": "Сериал",
  "calendar.seasonEpisode": "Сезон {{season}}, эпизод {{episode}}",
  "calendar.episode": "Эпизод {{episode}}",
  "calendar.library": "Библиотека: {{library}}",
  "calendar.typeLabel": "Тип: {{type}}",
  "calendar.notMonitored": "(Не отслеживается)",
  "calendar.dateAtTime": "{{date}} в {{time}}",
  "oauth.ui.OAuth_application_updated": "Приложение OAuth обновлено.",
  "oauth.ui.OAuth_application_created": "Приложение OAuth создано.",
  "oauth.ui.Unable_to_copy_the_OAuth_client_ID":
    "Не удалось скопировать ID клиента OAuth.",
  "oauth.ui.OAuth_client_ID_copied": "ID клиента OAuth скопирован.",
  "oauth.ui.Unable_to_copy_the_Jellyfin_callback_URL":
    "Не удалось скопировать URL обратного вызова Jellyfin.",
  "oauth.ui.Jellyfin_callback_URL_copied":
    "URL обратного вызова Jellyfin скопирован.",
  "oauth.ui.Enter_a_valid_public_HTTPS_Jellyfin_base_URL":
    "Введите корректный публичный базовый URL Jellyfin с HTTPS.",
  "oauth.ui.Multiple_custom_OAuth_clients_use_this_exact_Jellyfin_callback_Remove_duplicates_before_continuing":
    "Несколько пользовательских клиентов OAuth используют этот URL обратного вызова Jellyfin. Прежде чем продолжить, удалите дубликаты.",
  "oauth.ui.Jellyfin_plugin_OAuth_client_is_already_configured":
    "Клиент OAuth для плагина Jellyfin уже настроен.",
  "oauth.ui.The_create_result_is_uncertain_Reloading_OAuth_applications_before_another_create_is_allowed":
    "Результат создания неизвестен. Перед повторным созданием необходимо заново загрузить список приложений OAuth.",
  "oauth.ui.Jellyfin_plugin_OAuth_client_created":
    "Клиент OAuth для плагина Jellyfin создан.",
  "oauth.ui.OAuth_application_deleted_and_its_grants_revoked":
    "Приложение OAuth удалено; выданные ему разрешения отозваны.",
  "oauth.ui.reconciling_the_created_client": "сверка созданного клиента",
  "oauth.ui.ambiguous_multiple_custom_clients_use_this_exact_callback_remove_duplicates_before_continuing":
    "неоднозначность: несколько пользовательских клиентов используют этот URL обратного вызова; удалите дубликаты, прежде чем продолжить",
  "oauth.ui.not_configured_for_this_callback":
    "не настроен для этого URL обратного вызова",
  "oauth.ui.ambiguous_more_than_one_eligible_Jellyfin_connection_matches_this_public_URL":
    "неоднозначность: этому публичному URL соответствует несколько подходящих подключений Jellyfin",
  "oauth.ui.connection_status_unavailable": "состояние подключения недоступно",
  "oauth.ui.enter_a_public_HTTPS_URL_to_check_connection_binding":
    "введите публичный URL с HTTPS для проверки привязки подключения",
  "oauth.ui.requires_exactly_one_enabled_Jellyfin_connection_with_linking_an_API_key_and_this_exact_public_URL":
    "требуется ровно одно включённое подключение Jellyfin с привязкой учётных записей, API-ключом и в точности этим публичным URL",
  "oauth.ui.Enter_a_public_HTTPS_URL_to_derive_the_exact_callback":
    "Введите публичный URL с HTTPS, чтобы определить точный URL обратного вызова.",
  "oauth.ui.Uses_Scryer_s_built_in_native_callback_policy":
    "Используется встроенная политика URL обратного вызова Scryer.",
  "oauth.ui.Register_an_application": "Зарегистрировать приложение",
  "oauth.ui.Save_application": "Сохранить приложение",
  "oauth.ui.Create_application": "Создать приложение",
  "oauth.ui.Delete_application": "Удалить приложение",
  "rules.template.freeleech-bonus.title": "Предпочитать freeleech-релизы",
  "rules.template.freeleech-bonus.description":
    "Добавляет 500 баллов freeleech-релизам с индексаторов Torznab.",
  "rules.template.halfleech-bonus.title": "Предпочитать half-leech-релизы",
  "rules.template.halfleech-bonus.description":
    "Добавляет 200 баллов релизам half-leech, у которых учитывается 50% скачанного объёма.",
  "rules.template.well-seeded-bonus.title":
    "Предпочитать торренты с большим числом сидов",
  "rules.template.well-seeded-bonus.description":
    "Повышает оценку релизов с 10 и более сидами и понижает её при числе сидов менее 3. Используются сведения индексатора на момент обнаружения или захвата релиза.",
  "rules.template.block-exe-torrents.title": "Не допускать торренты с .exe",
  "rules.template.block-exe-torrents.description":
    "Резко понижает оценку торрентов, в названии которых указано расширение исполняемого файла.",
  "rules.template.prefer-web-dl.title": "Предпочитать WEB-DL вместо WEBRip",
  "rules.template.prefer-web-dl.description":
    "Добавляет 100 баллов релизам с источником WEB-DL.",
  "rules.template.prefer-hevc.title": "Предпочитать HEVC",
  "rules.template.prefer-hevc.description":
    "Добавляет 100 баллов релизам HEVC и вычитает 200 баллов у релизов x264 в 4K.",
  "rules.template.size-limits.title":
    "Предпочитать компактные релизы, понижать оценку слишком больших",
  "rules.template.size-limits.description":
    "Повышает оценку релизов менее 5 ГиБ; вычитает 10 000 баллов при размере более 100 ГиБ.",
  "rules.template.prefer-atmos-audio.title": "Предпочитать звук Atmos",
  "rules.template.prefer-atmos-audio.description":
    "Даёт небольшой бонус релизам с Atmos и небольшой штраф при отсутствии Atmos.",
  "rules.template.release-group-scores.title":
    "Предпочитать определённые релиз-группы",
  "rules.template.release-group-scores.description":
    "Повышает оценку релизов указанных групп. Чтобы понижать её, задайте отрицательные баллы.",
  "rules.template.block-old-releases.title":
    "Понижать оценку релизов старше года",
  "rules.template.block-old-releases.description":
    "Резко понижает оценку релизов, которым было более 365 дней на момент обнаружения или захвата. Возраст фиксируется при захвате: хранение файла в библиотеке не делает его старым.",
  "rules.template.block-obfuscated-retagged.title":
    "Понижать оценку релизов с запутанными или изменёнными именами",
  "rules.template.block-obfuscated-retagged.description":
    "Резко понижает оценку релизов с нормализованными признаками запутывания имён или изменения тегов.",
  "rules.template.block-hardcoded-subs.title":
    "Понижать оценку релизов с вшитыми субтитрами",
  "rules.template.block-hardcoded-subs.description":
    "Резко понижает оценку релизов с вшитыми в изображение субтитрами.",
  "rules.category.All": "Все",
  "rules.category.Torrent": "Торренты",
  "rules.category.Quality": "Качество",
  "rules.category.Size": "Размер",
  "rules.category.Audio": "Звук",
  "rules.category.Groups": "Группы",
  "rules.category.Penalties": "Штрафы",
  "jobs.builtin.LibraryScanMovies.name": "Сканирование библиотеки фильмов",
  "jobs.builtin.LibraryScanMovies.description":
    "Сканирование библиотеки фильмов и импорт отсутствующих файлов фильмов.",
  "jobs.builtin.LibraryScanSeries.name": "Сканирование библиотеки сериалов",
  "jobs.builtin.LibraryScanSeries.description":
    "Сканирование библиотеки сериалов и сверка файлов эпизодов.",
  "jobs.builtin.LibraryScanAnime.name": "Сканирование библиотеки аниме",
  "jobs.builtin.LibraryScanAnime.description":
    "Сканирование библиотеки аниме и сверка файлов эпизодов.",
  "jobs.builtin.BackgroundLibraryRefreshMovies.name":
    "Фоновое обновление библиотеки фильмов",
  "jobs.builtin.BackgroundLibraryRefreshMovies.description":
    "Облегчённое обновление библиотеки фильмов для обнаружения новых папок и файлов.",
  "jobs.builtin.BackgroundLibraryRefreshSeries.name":
    "Фоновое обновление библиотеки сериалов",
  "jobs.builtin.BackgroundLibraryRefreshSeries.description":
    "Облегчённое обновление библиотеки сериалов для обнаружения новых папок и добавленных файлов.",
  "jobs.builtin.BackgroundLibraryRefreshAnime.name":
    "Фоновое обновление библиотеки аниме",
  "jobs.builtin.BackgroundLibraryRefreshAnime.description":
    "Облегчённое обновление библиотеки аниме для обнаружения новых папок и добавленных файлов.",
  "jobs.builtin.ProwlarrSync.name": "Синхронизация Prowlarr",
  "jobs.builtin.ProwlarrSync.description":
    "Синхронизация включённых управляемых подключений Prowlarr и сверка дочерних индексаторов.",
  "jobs.builtin.RssSync.name": "Синхронизация RSS",
  "jobs.builtin.RssSync.description":
    "Получение RSS-лент включённых индексаторов и оценка новых релизов.",
  "jobs.builtin.SubtitleSearch.name": "Поиск субтитров",
  "jobs.builtin.SubtitleSearch.description":
    "Поиск отсутствующих субтитров для отслеживаемых медиафайлов.",
  "jobs.builtin.PluginRegistryRefresh.name": "Обновление каталога плагинов",
  "jobs.builtin.PluginRegistryRefresh.description":
    "Обновление метаданных каталога установленных плагинов.",
  "jobs.builtin.Housekeeping.name": "Очистка",
  "jobs.builtin.Housekeeping.description":
    "Очистка устаревших записей и удаление артефактов с истёкшим сроком хранения.",
  "jobs.builtin.HealthChecks.name": "Проверки состояния системы",
  "jobs.builtin.HealthChecks.description":
    "Выполнение настроенных проверок состояния системы.",
  "jobs.builtin.AutoBackup.name": "Автоматическое резервное копирование",
  "jobs.builtin.AutoBackup.description":
    "Создание ежедневной резервной копии с сохранением последних успешных автоматических копий.",
  "jobs.builtin.PendingReleaseProcessing.name": "Обработка ожидающих релизов",
  "jobs.builtin.PendingReleaseProcessing.description":
    "Задание совместимости: ожидающие релизы повторно оцениваются при синхронизации RSS.",
  "jobs.builtin.StagedNzbPrune.name": "Очистка временных NZB",
  "jobs.builtin.StagedNzbPrune.description":
    "Удаление временных NZB-артефактов с истёкшим сроком хранения.",
  "jobs.builtin.FullHashBackfill.name": "Вычисление полных хешей файлов",
  "jobs.builtin.FullHashBackfill.description":
    "Постепенное вычисление отсутствующих полных хешей файлов для обнаружения дубликатов и проверки перемещений.",
  "jobs.builtin.DiscoverySync.name": "Синхронизация рекомендаций",
  "jobs.builtin.DiscoverySync.description":
    "Проверка актуальности локальных рекомендаций и обновление снимков рекомендаций SMG.",
  "jobs.builtin.ArtworkEncoding.name": "Кодирование изображений",
  "jobs.builtin.ArtworkEncoding.description":
    "Ночное кодирование ожидающих изображений. Ручной запуск не ограничен ночным окном.",
  "jobs.builtin.TitleImageCacheRefresh.name":
    "Обновление кэша изображений материалов",
  "jobs.builtin.TitleImageCacheRefresh.description":
    "Обновление удалённых URL изображений из SMG и повторная обработка локальных изображений материалов.",
  "jobs.builtin.TitleDeletion.name": "Удаление материалов",
  "jobs.builtin.TitleDeletion.description":
    "Удаление выбранных материалов из каталога.",
  "jobs.builtin.TitleRename.name": "Переименование материалов",
  "jobs.builtin.TitleRename.description":
    "Переименование файлов выбранных материалов.",
  "jobs.builtin.MediaFileRescore.name": "Пересчёт оценок файлов",
  "jobs.builtin.MediaFileRescore.description":
    "Оценка выбранных файлов по действующим правилам.",
  "jobs.builtin.MediaFileDeletion.name": "Удаление медиафайла",
  "jobs.builtin.MediaFileDeletion.description":
    "Удаление медиафайла с диска и из каталога.",
  "jobs.builtin.RecycleBinRestore.name": "Восстановление из корзины",
  "jobs.builtin.RecycleBinRestore.description":
    "Восстановление файла из корзины в библиотеку.",
  "jobs.builtin.RecycleBinPurge.name": "Очистка корзины",
  "jobs.builtin.RecycleBinPurge.description":
    "Безвозвратное удаление выбранных файлов из корзины.",
  "jobs.builtin.AcquisitionSearch.name": "Поиск для загрузки",
  "jobs.builtin.AcquisitionSearch.description":
    "Интерактивный поиск загрузок для выбранных отсутствующих материалов или улучшений качества.",
  "jobs.builtin.ApplicationUpgrade.name": "Обновление приложения",
  "jobs.builtin.ApplicationUpgrade.description":
    "Загрузка, проверка и применение подписанного обновления приложения.",
  "jobs.builtin.MaintenanceRuleEvaluation.name": "Оценка правил обслуживания",
  "jobs.builtin.MaintenanceRuleEvaluation.description":
    "Оценка включённых правил обслуживания и сверка кандидатов жизненного цикла. Никогда не выполняет действия.",
  "jobs.builtin.LifecycleActionHandling.name":
    "Выполнение действий обслуживания",
  "jobs.builtin.LifecycleActionHandling.description":
    "Обработка готовых кандидатов обслуживания для активированных правил наблюдения с учётом ограничений экземпляра и полных повторных проверок безопасности.",
  "jobs.builtin.MediaServerSignalSync.name":
    "Синхронизация просмотров медиасерверов",
  "jobs.builtin.MediaServerSignalSync.description":
    "Чтение статуса просмотра для подтверждённых связанных учётных записей включённых медиасерверов и сохранение нормализованных сигналов просмотра.",
  "jobs.builtin.ListSync.name": "Синхронизация списков",
  "jobs.builtin.ListSync.description":
    "Чтение подписок на списки по расписанию, затем добавление материалов, создание запросов или запись материалов согласно настройкам каждого списка.",
  "jobs.builtin.LocationOperation.name": "Перемещение контента",
  "jobs.builtin.LocationOperation.description":
    "Перемещение контента и записей каталога между корневыми папками или библиотеками.",
  "jobs.builtin.CustomJob.name": "Пользовательское задание",
  "jobs.builtin.CustomJob.description":
    "Запуск пользовательского скрипта по его собственному расписанию.",
  "jobs.builtinText.Manual_only": "Только вручную",
  "jobs.builtinText.Every_2_hours": "Каждые 2 часа",
  "jobs.builtinText.Every_6_hours": "Каждые 6 часов",
  "jobs.builtinText.Every_8_hours": "Каждые 8 часов",
  "jobs.builtinText.Every_12_hours": "Каждые 12 часов",
  "jobs.builtinText.Every_24_hours": "Каждые 24 часа",
  "jobs.builtinText.Every_15_minutes": "Каждые 15 минут",
  "jobs.builtinText.Every_hour": "Каждый час",
  "jobs.builtinText.Daily_at_configured_local_time":
    "Ежедневно в настроенное местное время",
  "jobs.builtinText.Dynamic_discovery_evaluator_with_daily_backstop":
    "Динамическая проверка рекомендаций с обязательным ежедневным запуском",
  "jobs.builtinText.Every_30_minutes_02_00_06_00_host_local_time":
    "Каждые 30 минут, с 02:00 до 06:00 по местному времени сервера",
  "jobs.builtinText.Re_evaluated_during_RSS_sync":
    "Повторная оценка при синхронизации RSS",
  "jobs.builtinText.Scheduler_paced_per_indexer_15_minute_target":
    "По расписанию каждого индексатора, целевой интервал — 15 минут",
  "jobs.builtinText.Based_on_subtitle_settings_interval":
    "С интервалом из настроек субтитров",
  "jobs.builtinText.Encode_pending_artwork_Run_now_overrides_the_nightly_window_Waiting_for_pending_artwork":
    "Кодирование ожидающих изображений. Ручной запуск не ограничен ночным окном. Ожидание изображений для обработки.",
  "providerConfig.Known_provider": "Известный провайдер",
  "providerConfig.Base_URL": "Базовый URL",
  "providerConfig.API_Key": "API-ключ",
  "providerConfig.API_key": "API-ключ",
  "providerConfig.API_Path": "Путь API",
  "providerConfig.API_path": "Путь API",
  "providerConfig.Host": "Хост",
  "providerConfig.Hostname": "Имя хоста",
  "providerConfig.Port": "Порт",
  "providerConfig.Username": "Имя пользователя",
  "providerConfig.Password": "Пароль",
  "providerConfig.Category": "Категория",
  "providerConfig.Categories": "Категории",
  "providerConfig.Tags": "Теги",
  "providerConfig.Priority": "Приоритет",
  "providerConfig.Default": "По умолчанию",
  "providerConfig.Custom": "Другой",
  "providerConfig.None": "Нет",
  "providerConfig.Timeout": "Тайм-аут",
  "providerConfig.Request_timeout": "Тайм-аут запроса",
  "providerConfig.Use_SSL": "Использовать SSL",
  "providerConfig.Verify_SSL": "Проверять SSL",
  "providerConfig.Verify_SSL_certificate": "Проверять SSL-сертификат",
  "providerConfig.URL_Base": "Базовый путь URL",
  "providerConfig.URL_base": "Базовый путь URL",
  "providerConfig.URL_Path": "Путь URL",
  "providerConfig.Download_directory": "Каталог загрузок",
  "providerConfig.Completed_directory": "Каталог завершённых загрузок",
  "providerConfig.Polling_interval": "Интервал опроса",
  "providerConfig.Rate_limit": "Ограничение частоты запросов",
  "providerConfig.Search_limit": "Ограничение поиска",
  "providerConfig.Page_size": "Размер страницы",
  "providerConfig.Proxy": "Прокси",
  "providerConfig.Enable_RSS": "Включить RSS",
  "providerConfig.Enable_search": "Включить поиск",
  "providerConfig.Seed_ratio": "Коэффициент раздачи",
  "providerConfig.Seed_time": "Время раздачи",
  "providerConfig.Minimum_seeders": "Минимальное число сидов",
  "providerConfig.about": "О поле «{{label}}»",
  "providerConfig.browse": "Обзор: {{label}}",
  "providerConfig.select": "Выберите: {{label}}",
  "jobs.artworkWindow":
    "Ежедневно с {{start}} до {{end}} по местному времени сервера; сейчас на сервере {{clock}}",
  "uiAttr.Replace_download": "Заменить загрузку",
  "uiAttr.Keep_current": "Оставить текущую",
  "uiAttr.Revoke_key": "Отозвать ключ",
  "uiAttr.Cancel": "Отмена",
  "uiAttr.All": "Все",
  "uiAttr.Global": "Глобально",
  "uiAttr.Apply_update": "Применить обновление",
  "uiAttr.Uninstall": "Удалить",
  "uiAttr.Back": "Назад",
  "permissions.MANAGE_USERS": "Управление пользователями",
  "permissions.MANAGE_PERMISSIONS": "Управление разрешениями",
  "permissions.MANAGE_SYSTEM_SETTINGS": "Управление системными настройками",
  "permissions.MANAGE_CATALOG_SETTINGS": "Управление настройками каталогов",
  "permissions.MANAGE_LISTS": "Управление списками",
  "permissions.VIEW": "Просмотр",
  "permissions.MANAGE_TITLES": "Управление материалами",
  "permissions.RESOLVE_IMPORTS": "Разрешение проблем импорта",
  "permissions.MANAGE_LIBRARY": "Управление библиотекой",
  "permissions.REQUEST": "Запрос материалов",
  "permissions.AUTO_APPROVE_REQUESTS": "Автоматическое одобрение запросов",
  "permissions.MANAGE_SUBTITLES": "Управление субтитрами",
  "permissions.selectAll": "Выбрать всё",
  "permissions.unselectAll": "Снять всё",
  "permissions.facet.MOVIE": "Фильмы",
  "permissions.facet.SERIES": "Сериалы",
  "permissions.facet.ANIME": "Аниме",
  "apiKeys.ui.Revoked": "Отозван",
  "apiKeys.ui.Expired": "Срок действия истёк",
  "apiKeys.ui.Policy_disabled": "Запрещён политикой",
  "apiKeys.ui.Active": "Активен",
  "apiKeys.ui.Enter_a_name_for_this_API_key":
    "Введите имя для этого API-ключа.",
  "apiKeys.ui.Clipboard_access_is_unavailable":
    "Доступ к буферу обмена недоступен.",
  "apiKeys.ui.API_key_copied": "API-ключ скопирован",
  "apiKeys.ui.Copy_API_key": "Скопировать API-ключ",
  "apiKeys.ui.30_days": "30 дней",
  "apiKeys.ui.90_days": "90 дней",
  "apiKeys.ui.1_year": "1 год",
  "apiKeys.ui.Never": "Никогда",
  "apiKeys.ui.Creating": "Создание…",
  "apiKeys.ui.Create_API_key": "Создать API-ключ",
  "apiKeys.ui.Revoking": "Отзыв…",
  "apiKeys.ui.Revoke": "Отозвать",
  "jobs.artworkWait.nightly":
    "Кодирование ожидающих изображений. Ручной запуск не ограничен ночным окном. Ожидание ночного окна.",
  "jobs.artworkWait.scan":
    "Кодирование ожидающих изображений. Ручной запуск не ограничен ночным окном. Ожидание завершения сканирования библиотек.",
};

export default ru;
