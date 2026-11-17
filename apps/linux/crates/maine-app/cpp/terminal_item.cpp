#include "terminal_item.h"

#include <QClipboard>
#include <QCoreApplication>
#include <QFontDatabase>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPainter>
#include <QPointer>
#include <QStyleHints>
#include <QWheelEvent>
#include <QtQml/qqml.h>
#include <algorithm>
#include <atomic>

// ---- Rust side (crates/maine-app/src/ffi.rs) ----
extern "C" {
MaineTerm *maine_term_attach(const char *session_id, void (*callback)(void *), void *user, void (*release)(void *));
void maine_term_free(MaineTerm *);
void maine_term_resize(MaineTerm *, uint16_t cols, uint16_t rows);
void maine_term_send(MaineTerm *, const uint8_t *bytes, size_t len);
void maine_term_paste(MaineTerm *, const char *text);
void maine_term_key(MaineTerm *, int key, const char *text, uint8_t mods);
bool maine_term_mouse(MaineTerm *, int button, int kind, uint32_t col, uint32_t row, uint8_t mods);
size_t maine_term_snapshot(MaineTerm *, MaineCell *out, size_t cap, MaineSnapshotInfo *info);
void maine_term_scroll(MaineTerm *, int lines);
void maine_term_selection_start(MaineTerm *, uint32_t col, uint32_t row, bool right, bool block);
void maine_term_selection_update(MaineTerm *, uint32_t col, uint32_t row, bool right);
void maine_term_selection_word(MaineTerm *, uint32_t col, uint32_t row);
void maine_term_selection_line(MaineTerm *, uint32_t col, uint32_t row);
void maine_term_selection_clear(MaineTerm *);
char *maine_term_selection_text(MaineTerm *);
char *maine_term_screen_text(MaineTerm *);
void maine_string_free(char *);
bool maine_term_next_event(MaineTerm *, int *kind, char **text, int *code);
bool maine_term_is_running(MaineTerm *);
void maine_term_note_input(MaineTerm *);
bool maine_term_agent_driven(MaineTerm *);
}

namespace {
constexpr uint16_t F_BOLD = 1, F_ITALIC = 2, F_UNDERLINE = 4, F_STRIKE = 8, F_WIDE = 16, F_SPACER = 32, F_SELECTED = 64,
                   F_CURSOR = 128;
constexpr uint32_t M_APP_CURSOR = 1, M_BRACKETED = 1 << 1, M_ALT_SCREEN = 1 << 2, M_FOCUS = 1 << 3,
                   M_MOUSE_CLICK = 1 << 4, M_MOUSE_DRAG = 1 << 5, M_MOUSE_MOTION = 1 << 6, M_ALT_SCROLL = 1 << 8;

/// Shared with the Rust reader thread. Only touched from the GUI thread, via
/// the queued functor below, except for the weak pointer copy.
struct UpdateHandle {
    QPointer<TerminalItem> item;
};

void onRustUpdate(void *user) {
    auto *handle = static_cast<UpdateHandle *>(user);
    QPointer<TerminalItem> item = handle->item;
    // The functor runs on the GUI thread, where deleting the item cannot race the check.
    QMetaObject::invokeMethod(
        QCoreApplication::instance(), [item]() { if (item) item->handleUpdate(); }, Qt::QueuedConnection);
}

void releaseHandle(void *user) { delete static_cast<UpdateHandle *>(user); }

QColor rgb(uint32_t c) { return QColor((c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff); }

uint8_t modBits(Qt::KeyboardModifiers m) {
    return (m & Qt::ShiftModifier ? 1 : 0) | (m & Qt::ControlModifier ? 2 : 0) | (m & Qt::AltModifier ? 4 : 0) |
           (m & Qt::MetaModifier ? 8 : 0);
}
} // namespace

extern "C" void maine_register_qml_types() {
    qmlRegisterType<TerminalItem>("Maine", 1, 0, "TerminalItem");
    qmlRegisterType<ClipboardHelper>("Maine", 1, 0, "ClipboardHelper");
}

void ClipboardHelper::copy(const QString &text) { QGuiApplication::clipboard()->setText(text); }
QString ClipboardHelper::paste() const { return QGuiApplication::clipboard()->text(); }
bool ClipboardHelper::grabWindow(QQuickWindow *window, const QString &path) { return window && window->grabWindow().save(path); }

TerminalItem::TerminalItem(QQuickItem *parent) : QQuickPaintedItem(parent) {
    setFlag(ItemHasContents, true);
    setFlag(ItemAcceptsInputMethod, true);
    setFlag(ItemIsFocusScope, true);
    setAcceptedMouseButtons(Qt::AllButtons);
    setAcceptHoverEvents(true);
    setAntialiasing(false);
    setRenderTarget(QQuickPaintedItem::FramebufferObject);
    updateMetrics();
}

TerminalItem::~TerminalItem() { detach(); }

void TerminalItem::detach() {
    if (m_term) {
        maine_term_free(m_term); // detaches the view; the session keeps running in the engine
        m_term = nullptr;
    }
    m_running = false;
    m_info = {};
}

void TerminalItem::setSessionId(const QString &id) {
    if (id == m_sessionId) return;
    detach();
    m_sessionId = id;
    emit sessionIdChanged();
    attachSession();
    emit runningChanged();
    update();
}

void TerminalItem::attachSession() {
    if (m_sessionId.isEmpty()) return;
    auto *handle = new UpdateHandle{QPointer<TerminalItem>(this)};
    const QByteArray id = m_sessionId.toUtf8();
    m_term = maine_term_attach(id.constData(), onRustUpdate, handle, releaseHandle);
    if (!m_term) return; // no such session (or it never started): the hook released `handle`
    m_running = maine_term_is_running(m_term);
    m_title.clear();
    // Push the real grid size to the PTY now; it may differ from the engine's initial guess.
    const int cols = std::max(1, int(width() / m_cellW)), rows = std::max(1, int(height() / m_cellH));
    if (width() >= 2 * m_cellW && height() >= 2 * m_cellH) {
        m_cols = cols;
        m_rows = rows;
        maine_term_resize(m_term, uint16_t(cols), uint16_t(rows));
        emit gridSizeChanged();
    }
    refreshSnapshot();
}

bool TerminalItem::running() const { return m_term && maine_term_is_running(m_term); }

void TerminalItem::updateMetrics() {
    m_font = m_fontFamily.isEmpty() ? QFontDatabase::systemFont(QFontDatabase::FixedFont) : QFont(m_fontFamily);
    m_font.setPixelSize(m_fontPixelSize);
    m_font.setStyleHint(QFont::Monospace);
    m_font.setKerning(false);
    QFontMetricsF fm(m_font);
    m_cellW = std::max<qreal>(1, fm.horizontalAdvance(QLatin1Char('M')));
    m_cellH = std::max<qreal>(1, std::ceil(fm.height()));
    m_ascent = fm.ascent();
}

void TerminalItem::setFontFamily(const QString &family) {
    if (family == m_fontFamily) return;
    m_fontFamily = family;
    updateMetrics();
    applyGridSize();
    emit fontChanged();
    update();
}

void TerminalItem::setFontPixelSize(int px) {
    px = std::clamp(px, 6, 72);
    if (px == m_fontPixelSize) return;
    m_fontPixelSize = px;
    updateMetrics();
    applyGridSize();
    emit fontChanged();
    update();
}

void TerminalItem::setUserDriven(bool v) {
    if (v == m_userDriven) return;
    m_userDriven = v;
    emit userDrivenChanged();
}

void TerminalItem::sendText(const QString &text) {
    if (!m_term || text.isEmpty()) return;
    const QByteArray bytes = text.toUtf8();
    maine_term_send(m_term, reinterpret_cast<const uint8_t *>(bytes.constData()), size_t(bytes.size()));
}

QString TerminalItem::screenText() {
    if (!m_term) return {};
    char *raw = maine_term_screen_text(m_term);
    QString text = QString::fromUtf8(raw);
    maine_string_free(raw);
    return text;
}

void TerminalItem::debugKey(int qtKey, const QString &text, int modifiers) {
    QKeyEvent ev(QEvent::KeyPress, qtKey, Qt::KeyboardModifiers(modifiers), text);
    keyPressEvent(&ev);
}

void TerminalItem::paste() {
    if (!m_term) return;
    maine_term_note_input(m_term);
    const QByteArray text = QGuiApplication::clipboard()->text().toUtf8();
    if (!text.isEmpty()) maine_term_paste(m_term, text.constData());
}

void TerminalItem::copy() {
    if (!m_term) return;
    char *raw = maine_term_selection_text(m_term);
    if (!raw) return;
    QGuiApplication::clipboard()->setText(QString::fromUtf8(raw));
    maine_string_free(raw);
}

void TerminalItem::setClipboardSelection(const QString &text) {
    if (QGuiApplication::clipboard()->supportsSelection()) QGuiApplication::clipboard()->setText(text, QClipboard::Selection);
}

void TerminalItem::handleUpdate() {
    if (!m_term) return;
    pollEvents();
    if (m_term && isVisible()) {
        refreshSnapshot();
        update();
    }
}

void TerminalItem::pollEvents() {
    int kind = 0, code = 0;
    char *text = nullptr;
    while (m_term && maine_term_next_event(m_term, &kind, &text, &code)) {
        const QString s = text ? QString::fromUtf8(text) : QString();
        maine_string_free(text);
        text = nullptr;
        switch (kind) {
        case 1:
            if (s != m_title) {
                m_title = s;
                emit titleChanged();
            }
            break;
        case 2:
            if (m_userDriven && !maine_term_agent_driven(m_term)) emit bell();
            break;
        case 3:
            if (m_userDriven && !maine_term_agent_driven(m_term)) QGuiApplication::clipboard()->setText(s);
            break;
        case 4:
            m_running = false;
            emit runningChanged();
            emit exited(code);
            break;
        }
    }
}

bool TerminalItem::refreshSnapshot() {
    if (!m_term) return false;
    const size_t need = size_t(m_cols) * size_t(m_rows);
    if (m_cells.size() < need) m_cells.resize(need);
    size_t n = maine_term_snapshot(m_term, m_cells.data(), m_cells.size(), &m_info);
    if (n == 0 && m_info.cols * m_info.rows > 0) {
        // The grid changed size underneath us (resize raced the snapshot).
        m_cols = int(m_info.cols);
        m_rows = int(m_info.rows);
        m_cells.resize(size_t(m_cols) * size_t(m_rows));
        n = maine_term_snapshot(m_term, m_cells.data(), m_cells.size(), &m_info);
    }
    return n > 0;
}

void TerminalItem::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) {
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    if (newGeometry.size() != oldGeometry.size()) applyGridSize();
}

void TerminalItem::applyGridSize() {
    // Not laid out yet: never shrink a PTY to a 1x1 grid.
    if (width() < 2 * m_cellW || height() < 2 * m_cellH) return;
    const int cols = std::max(1, int(width() / m_cellW));
    const int rows = std::max(1, int(height() / m_cellH));
    if (cols == m_cols && rows == m_rows) return;
    m_cols = cols;
    m_rows = rows;
    if (m_term) {
        maine_term_resize(m_term, uint16_t(cols), uint16_t(rows));
        refreshSnapshot();
    }
    emit gridSizeChanged();
    update();
}

void TerminalItem::paint(QPainter *p) {
    const QColor defaultBg = m_info.cols ? rgb(m_info.default_bg) : QColor(0x1e, 0x1e, 0x1e);
    p->fillRect(QRectF(0, 0, width(), height()), defaultBg);
    if (!m_term || m_info.cols == 0 || m_cells.size() < size_t(m_info.cols) * m_info.rows) return;

    const int cols = int(m_info.cols), rows = int(m_info.rows);
    QFont bold = m_font, italic = m_font, boldItalic = m_font;
    bold.setBold(true);
    italic.setItalic(true);
    boldItalic.setBold(true);
    boldItalic.setItalic(true);
    const QColor selBg(0x26, 0x4f, 0x78);
    const bool focused = hasActiveFocus();

    auto colors = [&](const MaineCell &c, QColor &fg, QColor &bg) {
        fg = rgb(c.fg);
        bg = rgb(c.bg);
        if (c.flags & F_SELECTED) bg = selBg;
        if ((c.flags & F_CURSOR) && focused && m_info.cursor_shape == 0) std::swap(fg, bg);
    };

    // Backgrounds, merged into runs.
    for (int r = 0; r < rows; ++r) {
        int c = 0;
        while (c < cols) {
            const MaineCell &first = m_cells[size_t(r) * cols + c];
            QColor fg, bg;
            colors(first, fg, bg);
            int end = c + 1;
            while (end < cols) {
                QColor fg2, bg2;
                colors(m_cells[size_t(r) * cols + end], fg2, bg2);
                if (bg2 != bg) break;
                ++end;
            }
            if (bg != defaultBg) p->fillRect(QRectF(c * m_cellW, r * m_cellH, (end - c) * m_cellW, m_cellH), bg);
            c = end;
        }
    }

    // Text: ASCII runs with identical style are drawn as one string; everything
    // else cell by cell so wide and fallback glyphs never shift the grid.
    for (int r = 0; r < rows; ++r) {
        const qreal baseline = r * m_cellH + m_ascent;
        int c = 0;
        while (c < cols) {
            const MaineCell &cell = m_cells[size_t(r) * cols + c];
            if (cell.flags & F_SPACER || cell.ch == ' ' || cell.ch == 0) {
                // blanks only matter for underline/strike
                if (!(cell.flags & (F_UNDERLINE | F_STRIKE))) {
                    ++c;
                    continue;
                }
            }
            QColor fg, bg;
            colors(cell, fg, bg);
            const uint16_t style = cell.flags & (F_BOLD | F_ITALIC | F_UNDERLINE | F_STRIKE);
            const bool ascii = cell.ch < 0x80 && cell.ch >= 0x20;
            int end = c + 1;
            if (ascii) {
                while (end < cols) {
                    const MaineCell &n = m_cells[size_t(r) * cols + end];
                    QColor fg2, bg2;
                    colors(n, fg2, bg2);
                    if (!(n.ch < 0x80 && n.ch >= 0x20) || (n.flags & (F_BOLD | F_ITALIC | F_UNDERLINE | F_STRIKE)) != style ||
                        fg2 != fg)
                        break;
                    ++end;
                }
            } else if (cell.flags & F_WIDE) {
                end = c + 2;
            }
            QString text;
            for (int i = c; i < end; ++i) {
                const uint32_t ch = m_cells[size_t(r) * cols + i].ch;
                if (ch >= 0x20) text += QString::fromUcs4(reinterpret_cast<const char32_t *>(&ch), 1);
            }
            p->setPen(fg);
            p->setFont((style & F_BOLD) ? ((style & F_ITALIC) ? boldItalic : bold) : ((style & F_ITALIC) ? italic : m_font));
            if (!text.trimmed().isEmpty()) p->drawText(QPointF(c * m_cellW, baseline), text);
            if (style & F_UNDERLINE) p->fillRect(QRectF(c * m_cellW, r * m_cellH + m_ascent + 1, (end - c) * m_cellW, 1), fg);
            if (style & F_STRIKE) p->fillRect(QRectF(c * m_cellW, r * m_cellH + m_cellH / 2, (end - c) * m_cellW, 1), fg);
            c = end;
        }
    }

    // Cursor shapes other than the inverted block.
    if (m_info.cursor_col >= 0) {
        const MaineCell &cc = m_cells[size_t(m_info.cursor_row) * cols + m_info.cursor_col];
        const QColor fg = rgb(m_info.default_fg);
        const QRectF cell(m_info.cursor_col * m_cellW, m_info.cursor_row * m_cellH, (cc.flags & F_WIDE ? 2 : 1) * m_cellW, m_cellH);
        if (!focused) {
            p->setPen(fg);
            p->setBrush(Qt::NoBrush);
            p->drawRect(cell.adjusted(0.5, 0.5, -0.5, -0.5));
        } else if (m_info.cursor_shape == 1) {
            p->fillRect(QRectF(cell.left(), cell.bottom() - 2, cell.width(), 2), fg);
        } else if (m_info.cursor_shape == 2) {
            p->fillRect(QRectF(cell.left(), cell.top(), 2, cell.height()), fg);
        } else if (m_info.cursor_shape == 3) {
            p->setPen(fg);
            p->setBrush(Qt::NoBrush);
            p->drawRect(cell.adjusted(0.5, 0.5, -0.5, -0.5));
        }
    }
}

// ---- keyboard ----

void TerminalItem::keyPressEvent(QKeyEvent *event) {
    if (!m_term) {
        event->ignore();
        return;
    }
    maine_term_note_input(m_term);
    const Qt::KeyboardModifiers m = event->modifiers();
    const int qk = event->key();
    // Terminal-emulator conventions: Ctrl+Shift+C/V, Shift+Insert, Shift+PageUp/Down.
    if ((m & Qt::ControlModifier) && (m & Qt::ShiftModifier)) {
        if (qk == Qt::Key_C) { copy(); return; }
        if (qk == Qt::Key_V) { paste(); return; }
        if (qk == Qt::Key_Plus || qk == Qt::Key_Equal) { setFontPixelSize(m_fontPixelSize + 1); return; }
        if (qk == Qt::Key_Underscore || qk == Qt::Key_Minus) { setFontPixelSize(m_fontPixelSize - 1); return; }
    }
    if ((m & Qt::ShiftModifier) && qk == Qt::Key_Insert) { paste(); return; }
    if ((m & Qt::ShiftModifier) && (qk == Qt::Key_PageUp || qk == Qt::Key_PageDown)) {
        maine_term_scroll(m_term, (qk == Qt::Key_PageUp ? 1 : -1) * std::max(1, m_rows - 1));
        return;
    }
    if (qk == Qt::Key_Shift || qk == Qt::Key_Control || qk == Qt::Key_Alt || qk == Qt::Key_Meta ||
        qk == Qt::Key_AltGr || qk == Qt::Key_CapsLock || qk == Qt::Key_NumLock || qk == Qt::Key_Super_L ||
        qk == Qt::Key_Super_R) {
        event->accept();
        return;
    }

    int key = 0;
    switch (qk) {
    case Qt::Key_Return: case Qt::Key_Enter: key = 1; break;
    case Qt::Key_Tab: key = 2; break;
    case Qt::Key_Backtab: key = 3; break;
    case Qt::Key_Backspace: key = 4; break;
    case Qt::Key_Escape: key = 5; break;
    case Qt::Key_Up: key = 6; break;
    case Qt::Key_Down: key = 7; break;
    case Qt::Key_Left: key = 8; break;
    case Qt::Key_Right: key = 9; break;
    case Qt::Key_Home: key = 10; break;
    case Qt::Key_End: key = 11; break;
    case Qt::Key_PageUp: key = 12; break;
    case Qt::Key_PageDown: key = 13; break;
    case Qt::Key_Insert: key = 14; break;
    case Qt::Key_Delete: key = 15; break;
    default:
        if (qk >= Qt::Key_F1 && qk <= Qt::Key_F24) key = 100 + (qk - Qt::Key_F1 + 1);
    }

    QString text = event->text();
    if (key == 0 && (m & Qt::ControlModifier)) {
        // Qt reports Ctrl+letter as a control character; the encoder wants the letter.
        if (qk >= Qt::Key_A && qk <= Qt::Key_Z) text = QString(QChar(qk)).toLower();
        else if (qk >= 0x20 && qk < 0x7f) text = QString(QChar(qk));
    }
    if (key == 0 && text.isEmpty()) {
        event->ignore();
        return;
    }
    if (key == 0 && text.size() == 1 && text[0].unicode() < 0x20 && !(m & Qt::ControlModifier)) {
        // stray control characters from layouts: send verbatim
        const QByteArray raw = text.toUtf8();
        maine_term_send(m_term, reinterpret_cast<const uint8_t *>(raw.constData()), size_t(raw.size()));
        event->accept();
        return;
    }
    // Shift is already in the text for printable keys; only pass it for named keys.
    uint8_t bits = modBits(m);
    if (key == 0) bits &= uint8_t(~1);
    const QByteArray utf8 = text.toUtf8();
    maine_term_key(m_term, key, utf8.constData(), bits);
    event->accept();
}

void TerminalItem::inputMethodEvent(QInputMethodEvent *event) {
    if (m_term) maine_term_note_input(m_term);
    if (!event->commitString().isEmpty()) sendText(event->commitString());
    event->accept();
}

QVariant TerminalItem::inputMethodQuery(Qt::InputMethodQuery query) const {
    switch (query) {
    case Qt::ImEnabled: return true;
    case Qt::ImCursorRectangle:
        return QRectF(std::max(0, m_info.cursor_col) * m_cellW, std::max(0, m_info.cursor_row) * m_cellH, m_cellW, m_cellH);
    case Qt::ImHints: return int(Qt::ImhNoPredictiveText | Qt::ImhNoAutoUppercase);
    default: return QQuickPaintedItem::inputMethodQuery(query);
    }
}

void TerminalItem::focusInEvent(QFocusEvent *event) {
    QQuickPaintedItem::focusInEvent(event);
    if (m_term && (m_info.modes & M_FOCUS)) maine_term_send(m_term, reinterpret_cast<const uint8_t *>("\x1b[I"), 3);
    update();
}

void TerminalItem::focusOutEvent(QFocusEvent *event) {
    QQuickPaintedItem::focusOutEvent(event);
    if (m_term && (m_info.modes & M_FOCUS)) maine_term_send(m_term, reinterpret_cast<const uint8_t *>("\x1b[O"), 3);
    update();
}

// ---- mouse ----

QPoint TerminalItem::cellAt(const QPointF &pos) const {
    return {std::clamp(int(pos.x() / m_cellW), 0, std::max(0, m_cols - 1)), std::clamp(int(pos.y() / m_cellH), 0, std::max(0, m_rows - 1))};
}

bool TerminalItem::sendMouse(int button, int kind, const QMouseEvent *event) {
    // Shift bypasses mouse reporting so the user can always select text.
    if (!m_term || (event->modifiers() & Qt::ShiftModifier)) return false;
    const QPoint c = cellAt(event->position());
    return maine_term_mouse(m_term, button, kind, uint32_t(c.x()), uint32_t(c.y()), modBits(event->modifiers()));
}

void TerminalItem::mousePressEvent(QMouseEvent *event) {
    forceActiveFocus(Qt::MouseFocusReason);
    if (!m_term) return;
    maine_term_note_input(m_term);
    if (event->button() == Qt::MiddleButton && !(m_info.modes & (M_MOUSE_CLICK | M_MOUSE_DRAG | M_MOUSE_MOTION))) {
        const QString sel = QGuiApplication::clipboard()->supportsSelection() ? QGuiApplication::clipboard()->text(QClipboard::Selection) : QString();
        if (!sel.isEmpty()) {
            const QByteArray b = sel.toUtf8();
            maine_term_paste(m_term, b.constData());
        }
        return;
    }
    const int button = event->button() == Qt::LeftButton ? 0 : event->button() == Qt::MiddleButton ? 1 : 2;
    if (sendMouse(button, 0, event)) { m_selecting = false; return; }
    if (event->button() != Qt::LeftButton) return;

    const QPoint c = cellAt(event->position());
    const bool right = std::fmod(event->position().x(), m_cellW) > m_cellW / 2;
    const int interval = QGuiApplication::styleHints()->mouseDoubleClickInterval();
    const bool again = m_clickTimer.isValid() && m_clickTimer.elapsed() < interval &&
                       (event->position() - m_lastClickPos).manhattanLength() < 8;
    m_clickCount = again ? m_clickCount + 1 : 1;
    m_clickTimer.restart();
    m_lastClickPos = event->position();
    if (m_clickCount == 2) maine_term_selection_word(m_term, uint32_t(c.x()), uint32_t(c.y()));
    else if (m_clickCount >= 3) { maine_term_selection_line(m_term, uint32_t(c.x()), uint32_t(c.y())); m_clickCount = 3; }
    else {
        maine_term_selection_start(m_term, uint32_t(c.x()), uint32_t(c.y()), right, event->modifiers() & Qt::AltModifier);
        m_pressCell = c;
        m_selectionRight = right;
    }
    m_selecting = true;
    handleUpdate();
}

void TerminalItem::mouseMoveEvent(QMouseEvent *event) {
    if (!m_term) return;
    const int button = event->buttons() & Qt::LeftButton ? 0 : event->buttons() & Qt::MiddleButton ? 1 : event->buttons() & Qt::RightButton ? 2 : 5;
    if (sendMouse(button, 2, event)) return;
    if (!m_selecting || m_clickCount > 1) return;
    const QPoint c = cellAt(event->position());
    maine_term_selection_update(m_term, uint32_t(c.x()), uint32_t(c.y()), std::fmod(event->position().x(), m_cellW) > m_cellW / 2);
    // Dragging above/below the viewport scrolls the history.
    if (event->position().y() < 0) maine_term_scroll(m_term, 1);
    else if (event->position().y() > height()) maine_term_scroll(m_term, -1);
    handleUpdate();
}

void TerminalItem::mouseReleaseEvent(QMouseEvent *event) {
    if (!m_term) return;
    const int button = event->button() == Qt::LeftButton ? 0 : event->button() == Qt::MiddleButton ? 1 : 2;
    if (sendMouse(button, 1, event)) return;
    if (m_selecting && event->button() == Qt::LeftButton) {
        m_selecting = false;
        char *raw = maine_term_selection_text(m_term);
        if (raw) {
            setClipboardSelection(QString::fromUtf8(raw));
            maine_string_free(raw);
        } else if (m_clickCount <= 1) {
            maine_term_selection_clear(m_term); // a plain click clears, like other terminals
            handleUpdate();
        }
    }
}

void TerminalItem::mouseDoubleClickEvent(QMouseEvent *event) { mousePressEvent(event); }

void TerminalItem::wheelEvent(QWheelEvent *event) {
    if (!m_term) return;
    const int delta = event->angleDelta().y();
    if (delta == 0) return;
    const int notches = std::max(1, std::abs(delta) / 120);
    const QPoint c = cellAt(event->position());
    const bool reporting = m_info.modes & (M_MOUSE_CLICK | M_MOUSE_DRAG | M_MOUSE_MOTION);
    if (reporting && !(event->modifiers() & Qt::ShiftModifier)) {
        for (int i = 0; i < notches; ++i)
            maine_term_mouse(m_term, delta > 0 ? 3 : 4, 0, uint32_t(c.x()), uint32_t(c.y()), modBits(event->modifiers()));
    } else if ((m_info.modes & M_ALT_SCREEN) && (m_info.modes & M_ALT_SCROLL)) {
        const char *seq = delta > 0 ? ((m_info.modes & M_APP_CURSOR) ? "\x1bOA" : "\x1b[A") : ((m_info.modes & M_APP_CURSOR) ? "\x1bOB" : "\x1b[B");
        for (int i = 0; i < notches * 3; ++i) maine_term_send(m_term, reinterpret_cast<const uint8_t *>(seq), 3);
    } else {
        maine_term_scroll(m_term, (delta > 0 ? 1 : -1) * 3 * notches);
    }
    event->accept();
}
