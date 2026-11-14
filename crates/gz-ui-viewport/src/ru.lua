-- The viewport's words in Russian: its actions, the pills and what they open, the stats pill,
-- the axis gizmo, Preferences > Viewport (docs/UI_DESIGN.md, section 9; the glossary is the
-- kit's, gz-ui/src/i18n/ru.lua).
--
-- Terms of this surface: select / selection - выделить / выделение (the Select mode and tool -
-- «Выделение»); frame (the camera) - показать; snap - привязка; bookmark - закладка; Game Lit -
-- игровое освещение; a view from an axis - «Сверху», «Спереди», ...
return {
  i18n = 1,
  lang = "ru",

  -- the group of the actions (the Command Palette, the Keyboard page) and the page of Preferences
  { "Viewport", "Вьюпорт" },

  -- the modes
  { "Mode", "Режим" },
  { "Select", "Выделение" },
  { "Place", "Размещение" },
  { "Paint", "Рисование" },
  { "Zones", "Зоны" },
  { "Select Mode", "Режим выделения" },
  { "Place Mode", "Режим размещения" },
  { "Paint Mode", "Режим рисования" },
  { "Zones Mode", "Режим зон" },
  { "Back to Select Mode", "Вернуться в режим выделения" },
  { "Select | Place | Paint | Zones (1, 2, 3, 4): what a click in the viewport does", "Выделение | Размещение | Рисование | Зоны (1, 2, 3, 4): что делает щелчок во вьюпорте" },

  -- why an action is off
  { "The camera is flying.", "Камера в режиме полёта." },
  { "Open a scene first.", "Сначала откройте сцену." },
  { "Select something first.", "Сначала выделите что-нибудь." },
  { "Something selected cannot take it.", "Это поддерживают не все выделенные объекты." },
  { "Select mode is up.", "Режим выделения уже включён." },

  -- the transform tools and their space
  { "Move", "Перемещение" },
  { "Rotate", "Поворот" },
  { "Scale", "Масштаб" },
  { "World / Local", "Мир / локально" },
  { "World", "Мир" },
  { "Local", "Локально" },

  -- the camera
  { "Frame Selection", "Показать выделенное" },
  { "Frame All", "Показать всё" },
  { "Perspective", "Перспектива" },
  { "Top", "Сверху", ctx = "Viewport" },
  { "Bottom", "Снизу", ctx = "Viewport" },
  { "Front", "Спереди", ctx = "Viewport" },
  { "Back", "Сзади", ctx = "Viewport" },
  { "Left", "Слева", ctx = "Viewport" },
  { "Right", "Справа", ctx = "Viewport" },
  { "Orthographic", "Ортографическая проекция" },
  { "The camera's view, framing, bookmarks", "Вид камеры, кадрирование, закладки" },
  { "Perspective: back to the free view", "Перспектива: вернуться к свободному виду" },
  { "Front view", "Вид спереди" },
  { "Back view", "Вид сзади" },
  { "Left view", "Вид слева" },
  { "Right view", "Вид справа" },
  { "Top view", "Вид сверху" },
  { "Bottom view", "Вид снизу" },
  { "Camera speed: {speed}", "Скорость камеры: {speed}" },
  { "Camera Bookmarks", "Закладки камеры" },
  { "Bookmark {n} (empty)", "Закладка {n} (пусто)" },
  { "Go to Bookmark 1", "Перейти к закладке 1" },
  { "Go to Bookmark 2", "Перейти к закладке 2" },
  { "Go to Bookmark 3", "Перейти к закладке 3" },
  { "Go to Bookmark 4", "Перейти к закладке 4" },
  { "Go to Bookmark 5", "Перейти к закладке 5" },
  { "Go to Bookmark 6", "Перейти к закладке 6" },
  { "Go to Bookmark 7", "Перейти к закладке 7" },
  { "Go to Bookmark 8", "Перейти к закладке 8" },
  { "Go to Bookmark 9", "Перейти к закладке 9" },
  { "Go to Camera Bookmark 1", "Перейти к закладке камеры 1" },
  { "Go to Camera Bookmark 2", "Перейти к закладке камеры 2" },
  { "Go to Camera Bookmark 3", "Перейти к закладке камеры 3" },
  { "Go to Camera Bookmark 4", "Перейти к закладке камеры 4" },
  { "Go to Camera Bookmark 5", "Перейти к закладке камеры 5" },
  { "Go to Camera Bookmark 6", "Перейти к закладке камеры 6" },
  { "Go to Camera Bookmark 7", "Перейти к закладке камеры 7" },
  { "Go to Camera Bookmark 8", "Перейти к закладке камеры 8" },
  { "Go to Camera Bookmark 9", "Перейти к закладке камеры 9" },
  { "Set Camera Bookmark 1", "Сохранить закладку камеры 1" },
  { "Set Camera Bookmark 2", "Сохранить закладку камеры 2" },
  { "Set Camera Bookmark 3", "Сохранить закладку камеры 3" },
  { "Set Camera Bookmark 4", "Сохранить закладку камеры 4" },
  { "Set Camera Bookmark 5", "Сохранить закладку камеры 5" },
  { "Set Camera Bookmark 6", "Сохранить закладку камеры 6" },
  { "Set Camera Bookmark 7", "Сохранить закладку камеры 7" },
  { "Set Camera Bookmark 8", "Сохранить закладку камеры 8" },
  { "Set Camera Bookmark 9", "Сохранить закладку камеры 9" },

  -- the selection as the viewport handles it
  { "Drop to Floor", "Опустить на пол" },
  { "Hide Selected", "Скрыть выделенное" },
  { "Show All Hidden", "Показать все скрытые" },
  { "Group", "Сгруппировать", ctx = "action" },

  -- the view modes and Game Lit
  { "Studio", "Студия" },
  { "Unlit", "Без освещения" },
  { "Game Lit", "Игровое освещение" },
  { "Wireframe", "Каркас" },
  { "How the scene is lit: Studio, Unlit, Game Lit, Wireframe", "Как освещена сцена: студия, без освещения, игровое освещение, каркас" },
  { "Rebake Roof Map", "Пересчитать карту крыш" },
  { "Weather", "Погода" },
  { "Default (a new game's first night)", "По умолчанию (первая ночь новой игры)" },
  { "Hour", "Час" },
  { "Exposure", "Экспозиция" },
  { "Game Tonemap", "Тонмаппинг игры" },
  { "Indoor Sky Light", "Свет неба в помещениях" },
  { "{n} {n:light|lights}, {weather} at {hour}", "{n} {n:источник света|источника света|источников света}, {weather}, {hour}" },
  { "the game has no weathers (configs\\environment\\weathers)", "В игре нет погоды (configs\\environment\\weathers)" },
  { "no weather '{name}' in the game's configs", "В конфигурации игры нет погоды «{name}»" },
  { "weather '{name}' has no hh:mm:ss sections", "У погоды «{name}» нет секций hh:mm:ss" },

  -- the Show pill
  { "Show", "Показать" },
  { "What the viewport draws over the scene", "Что вьюпорт рисует поверх сцены" },
  { "Grid", "Сетка" },
  { "Safe Frame", "Безопасная рамка" },
  { "Stats", "Статистика" },
  { "Spawn Shapes", "Фигуры точек спавна" },
  { "Spawn Names", "Имена точек спавна" },
  { "Light Names", "Имена источников света" },
  { "Glow Crosses", "Кресты свечений" },
  { "AI Map", "ИИ-сетка" },
  { "Fog", "Туман" },
  { "Player Start", "Точка старта игрока" },

  -- the snapping
  { "Grid Snap", "Привязка к сетке" },
  { "Angle Snap", "Привязка угла" },
  { "Scale Snap", "Привязка масштаба" },
  { "Surface Snap", "Привязка к поверхности" },
  { "Align to Surface", "Выравнивать по поверхности" },
  { "Snap to Vertices", "Привязка к вершинам" },
  { "Surface Snap's options", "Параметры привязки к поверхности" },
  { "The grid step a move goes by", "Шаг сетки при перемещении" },
  { "The angle a turn goes by", "Шаг угла при повороте" },
  { "The step scaling goes by", "Шаг масштабирования" },
  { "{v} m", "{v} м" },

  -- what the window's actions answer when they cannot
  { "nothing is selected to frame", "Нечего показать: ничего не выделено" },
  { "the scene has nothing to frame", "В сцене нечего показать" },
  { "the selection cannot move: not every selected object takes it", "Выделенное нельзя переместить: это поддерживают не все объекты" },
  { "the selection cannot rotate: not every selected object takes it", "Выделенное нельзя повернуть: это поддерживают не все объекты" },
  { "the selection cannot scale: not every selected object takes it", "Выделенное нельзя масштабировать: это поддерживают не все объекты" },
  { "The details were not painted.", "Растительность не нарисована." },

  -- a drag over the viewport
  { "Place {title}", "Разместить: {title}" },

  -- the stats pill
  { "FPS", "FPS" },
  { "Render FPS", "FPS рендера" },
  { "Triangles", "Треугольники" },
  { "Draw calls", "Вызовы отрисовки" },
  { "{a} of {b}", "{a} из {b}" },

  -- Preferences > Viewport
  { "Rendering", "Рендеринг" },
  { "Graphics", "Графика" },
  { "Sound", "Звук" },
  { "Render Scale", "Масштаб рендеринга" },
  { "The size the viewport renders at, against its size on the screen. Below 100 % draws faster and blurrier.", "Размер, с которым рисует вьюпорт, относительно его размера на экране. Ниже 100 % — быстрее, но размытее." },
  { "25 %", "25 %" },
  { "50 %", "50 %" },
  { "100 %", "100 %" },
  { "200 %", "200 %" },
  { "Real Time", "Реальное время" },
  { "The viewport draws every frame, not only when something it shows changes.", "Вьюпорт рисует каждый кадр, а не только когда меняется то, что он показывает." },
  { "Graphics Adapter", "Видеоадаптер" },
  { "The graphics adapter the editor draws with. Takes effect the next time the editor starts.", "Видеоадаптер, которым рисует редактор. Вступает в силу при следующем запуске редактора." },
  { "Use the Next Adapter", "Взять следующий адаптер" },
  { "Uses {active}; the next start uses {wanted}.", "Сейчас: {active}; при следующем запуске: {wanted}." },
  { "unknown", "неизвестно", ctx = "adapter" },
  { "the system's choice", "по выбору системы" },
  { "GPU Preference", "Предпочтительный GPU" },
  { "Which GPU Windows gives the editor on a computer with two. Takes effect the next time the editor starts.", "Какой GPU Windows выделяет редактору на компьютере с двумя видеокартами. Вступает в силу при следующем запуске редактора." },
  { "Automatic", "Автоматически" },
  { "High Performance", "Высокая производительность" },
  { "Power Saving", "Энергосбережение" },
  { "Mute Sounds", "Без звука" },
  { "The scene's sound sources stay silent in the viewport.", "Источники звука сцены не звучат во вьюпорте." },

  -- the transform tools
  { "Next Transform Tool", "Следующий инструмент трансформации" },

  -- camera bookmarks
  { "This camera bookmark keeps no camera yet: Ctrl and its digit keep one.", "В этой закладке камеры ещё ничего нет: сохраните камеру сочетанием Ctrl и цифры." },
}
