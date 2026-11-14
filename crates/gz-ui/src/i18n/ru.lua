-- The kit's words in Russian, and the glossary every catalog of the editor keeps to
-- (docs/UI_DESIGN.md, section 9). An entry is { "English as the code says it", "Russian" }, with
-- ctx = "<context>" where the same English means something else there.
--
-- How the Russian reads
--   A professional localization, never word for word: say what the English means the way Russian
--   software says it. Shorter is better; Russian runs ~25% longer than English already.
--   Buttons and menu commands: an infinitive or a noun ("Сохранить", "Собрать модуль", "Отмена").
--   Titles, headings, labels, tabs, columns: sentence case, never Title Case ("Настройки модуля").
--   Instructions: the polite imperative ("Выберите папку"); "вы" lowercase; no "пожалуйста".
--   Errors: what happened, then what to do; "Не удалось ..." without "!"; no blame.
--   Quotes «ёлочки» („лапки“ inside); ё everywhere; a spaced em dash " — "; "…" for an ellipsis
--   (a menu row that opens a dialog keeps its "..." as "…").
--   Numbers stay as the code gives them; units with a space: "5 с", "12 м", "60 fps".
--   Key names stay as printed on the keys: Ctrl, Shift, Alt, Enter, Esc, Del, Tab, F5.
--   Names of the editor's own surfaces in a sentence take «»: "откройте «Настройки модуля»".
--   Product and service names stay as they are: Grizzly Editor, Dead Air, X-Ray, GitHub,
--   VirusTotal, JSGME, LTX, Lua, LuaJIT, MCP, PDA (in the UI: КПК), Windows, Explorer
--   (in a sentence: Проводник).
--
-- Glossary (English - Russian). One concept, one term, everywhere.
--   action - действие                     level - уровень
--   AI map - ИИ-сетка                     library - библиотека
--   asset - ассет                         light (an object) - источник света
--   author key - ключ автора              lighting (a build step) - освещение
--   base scene - базовая сцена            Level Settings - настройки уровня
--   behaviour graph - граф поведения      light animation - анимация света
--   bookmark - закладка                   loading screen - экран загрузки
--   build (noun) - сборка                 manifest - манифест
--   build (verb) - собрать                menu - меню
--   Build Mod - Собрать мод               mod - мод
--   Build Into Game - Собрать в игру      Mod Browser - браузер модов
--   Build to Folder - Собрать в папку     Mod Settings - настройки мода
--   catalog (Mod Browser's) - каталог     mode (game mode) - режим
--   chooser - выбор                       module - модуль
--   character - персонаж                  More (a fold) - Ещё
--   character template - шаблон персонажа node - узел
--   check (verb) - проверить              object - объект
--   chip (status bar) - индикатор         Outliner - структура сцены
--   Command Palette - палитра команд      Output Log - журнал
--   commandlet - коммандлет               package (noun) - пакет
--   Content Browser - браузер контента    Package Release - Подготовить релиз
--   context menu - контекстное меню       panel - панель
--   descriptor - дескриптор               PDA interface - интерфейс КПК
--   Details - свойства                    pin - пин
--   detail objects - детейл-объекты       Place (panel, mode) - размещение
--   dialog - диалог (title: окно)         Play - Играть
--   draft - черновик                      player start - точка старта игрока
--   drawer - выдвижная панель             preview - предпросмотр
--   export (verb) - экспортировать        problem - проблема
--   Export Flat - Экспорт в gamedata      project - проект
--   field - поле                          Publish Release - Опубликовать релиз
--   file - файл                           quest - квест
--   flow (a pin) - поток                  release - релиз
--   folder - папка                        repository - репозиторий
--   foliage - растительность              role - роль
--   frame (a comment frame) - рамка       scene - сцена
--   game - игра                           section (LTX) - секция
--   game graph - граф игры                shape - фигура
--   gizmo - манипулятор                   snapshot - снимок
--   graph - граф                          spawn point - точка спавна
--   group (of objects) - группа           spawn section - секция спавна
--   job - задача                          stage (of a job) - этап
--   kit (starting kit) - снаряжение       Start page - начальная страница
--   layer (painted) - слой                step (of a build) - шаг
--   stale (a build step) - устарел        Task Center - центр задач
--   tab - вкладка                         tag - тег
--   task (of a quest) - задача квеста     template - шаблон
--   texture - текстура                    tile - плитка
--   toolbar - панель инструментов         trader - торговец
--   undo / redo - отменить / повторить    variable - переменная
--   viewport - вьюпорт                    volume - объём
--   way path - путь                       way point - точка пути
--   World Rules - правила мира            zone - зона
return {
  i18n = 1,
  lang = "ru",

  -- buttons and words every surface shares
  { "OK", "ОК" },
  { "Cancel", "Отмена" },
  { "Close", "Закрыть" },
  { "Save", "Сохранить" },
  { "Save All", "Сохранить все" },
  { "Don't Save", "Не сохранять" },
  { "Delete", "Удалить" },
  { "Remove", "Убрать" },
  { "Add", "Добавить" },
  { "Clear", "Очистить" },
  { "Dismiss", "Скрыть" },
  { "Yes", "Да" },
  { "No", "Нет" },
  { "None", "Нет" },
  { "All", "Все" },
  { "Search", "Поиск" },
  { "More", "Ещё" },
  { "More options", "Другие параметры" },
  { "Back", "Назад" },
  { "{name} Options", "Параметры: {name}" },
  { "Confirm", "Подтверждение" },
  { "Error", "Ошибка" },
  { "Warning", "Предупреждение" },
  { "Information", "Сведения" },
  { "Result", "Результат" },
  { "Unsaved Changes", "Несохранённые изменения" },
  { "Working...", "Выполняется…" },
  { "Multiple values", "Разные значения" },
  { "Reset to default", "Сбросить к значению по умолчанию" },
  { "Point at a property to see what it does.", "Наведите указатель на свойство, чтобы узнать, что оно делает." },
  { "No folder selected", "Папка не выбрана" },
  { "No file selected", "Файл не выбран" },
  { "Browse for a folder", "Выбрать папку" },
  { "Browse for a file", "Выбрать файл" },
  { "Sort by this column", "Сортировать по этому столбцу" },
  { "Recent", "Недавние" },
  { "Favourites", "Избранное" },
  { "Nothing to pick", "Выбирать не из чего" },
  { "No matches", "Ничего не найдено" },
  { "Take it out of the favourites", "Убрать из избранного" },
  { "Add it to the favourites", "Добавить в избранное" },
  { "Remove this row", "Удалить строку" },
  { "Drag to resize, double-click to reset", "Перетащите, чтобы изменить размер; двойной щелчок — сбросить" },
  { "Drop it here", "Перетащите сюда" },
  { "Not shipped", "Не входит в сборку" },
  { "Deleted for good - this cannot be undone.", "Удаляется безвозвратно — это действие нельзя отменить." },
  { "Changes are lost if they are not saved.", "Несохранённые изменения будут потеряны." },
  { "Save changes to {one}?", "Сохранить изменения в «{one}»?" },
  { "Save changes to these items?", "Сохранить изменения в этих элементах?" },
  { "and {n} more", "и ещё {n}" },
  { "and {n} more {n:warning|warnings}", "и ещё {n} {n:предупреждение|предупреждения|предупреждений}" },

  -- words many surfaces share (added from the areas' work)
  { "Copy", "Копировать" },
  { "Create", "Создать" },
  { "Change", "Изменить" },
  { "Change...", "Изменить…" },
  { "Reset", "Сбросить" },
  { "Reset All", "Сбросить все" },
  { "Hide", "Скрыть" },
  { "Later", "Позже" },
  { "Action", "Действие" },
  { "Actions", "Действия" },
  { "Keys", "Клавиши" },
  { "Version", "Версия" },
  { "Folder", "Папка" },
  { "General", "Общие" },
  { "Language", "Язык" },
  { "Missing", "Отсутствует" },
  { "On", "Вкл." },
  { "Off", "Выкл." },
  { "Auto", "Авто" },
  { "Open", "Открыть" },
  { "Rename", "Переименовать" },
  { "Cut", "Вырезать" },
  { "Paste", "Вставить" },
  { "Refresh", "Обновить" },
  { "Select All", "Выделить всё" },
  { "Clear Selection", "Снять выделение" },
  { "Replace", "Заменить" },
  { "Place in Level", "Разместить на уровне" },
  { "Source", "Источник" },
  { "Textures", "Текстуры" },
  { "Interface", "Интерфейс" },
  { "Game Folder", "Папка игры" },
  { "Forward", "Вперёд" },
  { "Find", "Найти" },
  { "Find Next", "Найти далее" },
  { "Find Previous", "Найти предыдущее" },
  { "Duplicate", "Дублировать" },
  { "Undo", "Отменить" },
  { "Redo", "Повторить" },
  { "Apply", "Применить" },
  { "Enable", "Включить" },
  { "Disable", "Отключить" },
  { "Expand", "Развернуть" },
  { "Collapse", "Свернуть" },
  { "Move Up", "Переместить вверх" },
  { "Move Down", "Переместить вниз" },
  { "Move up", "Переместить вверх" },
  { "Move down", "Переместить вниз" },
  { "Default", "По умолчанию" },
  { "Zoom In", "Приблизить" },
  { "Zoom Out", "Отдалить" },
  { "Copy All", "Копировать всё" },
  { "Keyboard Shortcuts", "Сочетания клавиш" },
  { "Bookmarks", "Закладки" },
  { "Open a project first.", "Сначала откройте проект." },
  { "The command failed.", "Команда не выполнена." },
  { "Stopped.", "Остановлено." },
  -- sizes
  { "{n} {n:byte|bytes}", "{n} {n:байт|байта|байт}" },
  { "{n} KB", "{n} КБ" },
  { "{n} MB", "{n} МБ" },
  { "{n} GB", "{n} ГБ" },
  -- units
  { "s", "с" },
  { "min", "мин" },
  { "m/s", "м/с" },
  { "pt", "пт" },

  -- nouns several surfaces name
  { "Position", "Положение" },
  { "Volume", "Громкость" },
  { "Model", "Модель" },
  { "Profile", "Профиль" },
  { "Money", "Деньги" },
  { "Squad", "Отряд" },
  { "Item", "Предмет" },
  { "Trader", "Торговец" },
  { "Leads To", "Куда ведёт" },

  -- the theme picker (Preferences > Interface > Theme)
  { "Search themes", "Поиск тем" },
  { "Dark", "Тёмные", ctx = "pick.group" },
  { "Light", "Светлые", ctx = "pick.group" },
}
