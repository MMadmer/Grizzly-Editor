-- The shell's words in Russian: the caption, the menus, the tabs, the status bar, the Task Center,
-- the Command Palette (docs/UI_DESIGN.md, section 9; the glossary is the kit's, gz-ui/src/i18n/ru.lua).
return {
  i18n = 1,
  lang = "ru",

  { "System ({lang})", "Как в системе ({lang})" },

  -- the groups of the actions (the Command Palette, the Keyboard page)
  { "File", "Файл" },
  { "Help", "Справка" },
  { "Mod", "Мод" },
  { "View", "Вид" },
  { "Graph", "Граф" },
  { "Edit", "Правка", ctx = "group" },
  { "Window", "Окно" },
  { "Keyboard", "Клавиатура" },

  -- the caption's menus by their names
  { "File", "Файл", ctx = "menu" },
  { "Edit", "Правка", ctx = "menu" },
  { "Mod", "Мод", ctx = "menu" },
  { "Libraries", "Библиотеки", ctx = "menu" },
  { "Window", "Окно", ctx = "menu" },
  { "Help", "Справка", ctx = "menu" },
  { "Project", "Проект", ctx = "menu" },
  { "Menu", "Меню" },
  { "Menus", "Меню" },

  -- the shell's own actions
  { "About Grizzly Editor", "О программе Grizzly Editor" },
  { "Close Tab", "Закрыть вкладку" },
  { "Command Palette...", "Палитра команд…" },
  { "Content Drawer", "Панель контента" },
  { "Log Drawer", "Панель журнала" },
  { "Next Tab", "Следующая вкладка" },
  { "Previous Tab", "Предыдущая вкладка" },
  { "Open Project Folder", "Открыть папку проекта" },
  { "Preferences", "Настройки" },
  { "Quit", "Выход" },
  { "Reset Layout", "Сбросить раскладку" },
  { "Stop the Running Job", "Остановить текущую задачу" },
  { "Switch Project...", "Сменить проект…" },
  { "No job runs now.", "Сейчас нет выполняемых задач." },
  { "Nothing on this tab to save.", "На этой вкладке нечего сохранять." },
  { "The Level tab stays open.", "Вкладка «Уровень» не закрывается." },
  { "This tab has nothing to do with it.", "На этой вкладке это действие недоступно." },
  { "Run", "Выполнить", ctx = "palette" },

  -- a project card's menu (contract.rs)
  { "Delete...", "Удалить…" },
  { "Remove from List", "Убрать из списка" },

  -- the caption and the window's buttons
  { "Minimize", "Свернуть" },
  { "Maximize", "Развернуть" },
  { "Open: picking it closes it", "Открыта: выбор закроет её" },
  { "Closed: picking it opens it on Level", "Закрыта: выбор откроет её на вкладке «Уровень»" },

  -- the tabs
  { "Open tabs", "Открытые вкладки" },
  { "{n} more {n:tab|tabs}", "Ещё {n} {n:вкладка|вкладки|вкладок}" },
  { "Close Others", "Закрыть остальные" },
  { "Close All", "Закрыть все" },
  { "Close Saved", "Закрыть сохранённые" },
  { "Tab", "Вкладка" },
  { "Panel", "Панель" },
  { "The scene has unsaved changes.", "В сцене есть несохранённые изменения." },
  { "Unsaved changes.", "Есть несохранённые изменения." },
  { "File not found.", "Файл не найден." },
  { "File not found", "Файл не найден" },
  { "This tab stopped on an internal error", "Вкладка остановлена из-за внутренней ошибки" },
  { "{label} has unsaved changes.", "В «{label}» есть несохранённые изменения." },
  { "No level editor is installed in this shell.", "В эту оболочку не встроен редактор уровней." },

  -- the status bar
  { "MCP is on: agents drive the editor over 127.0.0.1.", "MCP включён: агенты управляют редактором через 127.0.0.1." },
  { "{n} {n:warning|warnings} since the project opened. Opens the Output Log.", "{n} {n:предупреждение|предупреждения|предупреждений} с открытия проекта. Открывает журнал." },
  { "{n} {n:error|errors} since the project opened. Opens the Output Log.", "{n} {n:ошибка|ошибки|ошибок} с открытия проекта. Открывает журнал." },
  { "{n} {n:task|tasks}", "{n} {n:задача|задачи|задач}" },
  { "The last jobs and how they ended", "Последние задачи и чем они закончились" },
  { "Open the Content Browser over this tab", "Открыть браузер контента поверх вкладки" },
  { "Open the Output Log over this tab", "Открыть журнал поверх вкладки" },
  { "Opens the Output Log", "Открывает журнал" },
  { "Click to open the Output Log.", "Щёлкните, чтобы открыть журнал." },
  { "Link the game this project is for: game.link over MCP.", "Привяжите игру, для которой этот проект: game.link через MCP." },
  { "Open a project with project.open over MCP, or start the editor with the project file.", "Откройте проект командой project.open через MCP или запустите редактор с файлом проекта." },
  { "Close the drawer (Esc)", "Закрыть панель (Esc)" },

  -- the Command Palette
  { "Run a command, open a panel or go to a tab", "Выполнить команду, открыть панель или перейти на вкладку" },
  { "No action, panel or tab has these words in its name.", "Ни у одного действия, панели или вкладки нет этих слов в названии." },
  { "{n} {n:result|results}", "{n} {n:результат|результата|результатов}" },

  -- the Task Center's card
  { "Hide. The job goes on, and the chip in the status bar brings this back.", "Скрыть. Задача продолжится, а индикатор в строке состояния вернёт карточку." },
  { " · about {eta} left", " · осталось около {eta}" },
  { "Stopping...", "Останавливается…" },
  { "It stops at the next step.", "Остановится на следующем шаге." },
  { "Recent Tasks", "Недавние задачи" },
  { "{title} - {result}, {ago} ago", "{title} — {result}, {ago} назад" },

  -- View > Render > GPU
  { "Driver Hint: Automatic", "Подсказка драйверу: автоматически" },
  { "Driver Hint: High Performance", "Подсказка драйверу: высокая производительность" },
  { "Driver Hint: Power Saving", "Подсказка драйверу: энергосбережение" },

  -- what a job that ended came to (tasks.rs)
  { "Done in {took}", "Готово за {took}" },
  { "Failed: {why}", "Ошибка: {why}" },
  { "Stopped", "Остановлено" },
  { "it stopped on an error", "остановлено из-за ошибки" },

  -- the tab's file in the Content Browser and in Explorer (Ctrl+B)
  { "The file is outside the project.", "Файл находится вне проекта." },
  { "This tab shows no file of the project.", "На этой вкладке нет файла проекта." },
}
