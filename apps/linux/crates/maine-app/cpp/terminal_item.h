#pragma once

#include <QElapsedTimer>
#include <QObject>
#include <QFont>
#include <QPointF>
#include <QQuickPaintedItem>
#include <QQuickWindow>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <vector>

extern "C" {
struct MaineTerm;
struct MaineCell {
    uint32_t ch, fg, bg;
    uint16_t flags, pad;
};
struct MaineSnapshotInfo {
    uint32_t cols, rows;
    int32_t cursor_col, cursor_row;
    uint32_t cursor_shape, display_offset, history, modes, default_fg, default_bg;
};
}

/// Paints a terminal grid owned by `maine-term` (Rust) and turns Qt input into
/// bytes for the child. All emulation lives in Rust; this class only draws
/// cells and forwards keys, mouse, wheel, paste and IME text.
class TerminalItem : public QQuickPaintedItem {
    Q_OBJECT
    /// The engine session (UUID) this view shows. Views only attach: the
    /// process and its scrollback belong to the engine, never to this item.
    Q_PROPERTY(QString sessionId READ sessionId WRITE setSessionId NOTIFY sessionIdChanged)
    Q_PROPERTY(QString title READ title NOTIFY titleChanged)
    Q_PROPERTY(bool running READ running NOTIFY runningChanged)
    Q_PROPERTY(int cols READ cols NOTIFY gridSizeChanged)
    Q_PROPERTY(int rows READ rows NOTIFY gridSizeChanged)
    Q_PROPERTY(QString fontFamily READ fontFamily WRITE setFontFamily NOTIFY fontChanged)
    Q_PROPERTY(int fontPixelSize READ fontPixelSize WRITE setFontPixelSize NOTIFY fontChanged)
    /// False while an agent drives the session: OSC 52 and the bell are ignored.
    Q_PROPERTY(bool userDriven READ userDriven WRITE setUserDriven NOTIFY userDrivenChanged)

public:
    explicit TerminalItem(QQuickItem *parent = nullptr);
    ~TerminalItem() override;

    Q_INVOKABLE void sendText(const QString &text);
    Q_INVOKABLE void paste();
    Q_INVOKABLE void copy();
    Q_INVOKABLE QString screenText();
    /// Test hook: delivers a synthetic key press through the real key handler.
    Q_INVOKABLE void debugKey(int qtKey, const QString &text, int modifiers);

    QString sessionId() const { return m_sessionId; }
    void setSessionId(const QString &id);
    QString title() const { return m_title; }
    bool running() const;
    int cols() const { return m_cols; }
    int rows() const { return m_rows; }
    QString fontFamily() const { return m_fontFamily; }
    void setFontFamily(const QString &family);
    int fontPixelSize() const { return m_fontPixelSize; }
    void setFontPixelSize(int px);
    bool userDriven() const { return m_userDriven; }
    void setUserDriven(bool v);

    void paint(QPainter *painter) override;

    /// Called on the GUI thread after the Rust side signalled new output.
    void handleUpdate();

signals:
    void sessionIdChanged();
    void titleChanged();
    void runningChanged();
    void gridSizeChanged();
    void fontChanged();
    void userDrivenChanged();
    void exited(int code);
    void bell();

protected:
    void keyPressEvent(QKeyEvent *event) override;
    void inputMethodEvent(QInputMethodEvent *event) override;
    QVariant inputMethodQuery(Qt::InputMethodQuery query) const override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void wheelEvent(QWheelEvent *event) override;
    void focusInEvent(QFocusEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private:
    void attachSession();
    void detach();
    void updateMetrics();
    void applyGridSize();
    void pollEvents();
    bool refreshSnapshot();
    QPoint cellAt(const QPointF &pos) const;
    bool sendMouse(int button, int kind, const QMouseEvent *event);
    void setClipboardSelection(const QString &text);

    MaineTerm *m_term = nullptr;
    QString m_sessionId;
    std::vector<MaineCell> m_cells;
    MaineSnapshotInfo m_info{};
    QString m_title;
    QString m_fontFamily;
    int m_fontPixelSize = 15;
    QFont m_font;
    qreal m_cellW = 9, m_cellH = 18, m_ascent = 14;
    int m_cols = 80, m_rows = 24;
    bool m_userDriven = true;
    bool m_running = false;
    bool m_selecting = false;
    bool m_selectionRight = false;
    QPoint m_pressCell{-1, -1};
    QElapsedTimer m_clickTimer;
    int m_clickCount = 0;
    QPointF m_lastClickPos;
};

/// Clipboard access for QML (the sidebar copies credentials with it).
class ClipboardHelper : public QObject {
    Q_OBJECT
public:
    using QObject::QObject;
    Q_INVOKABLE void copy(const QString &text);
    Q_INVOKABLE QString paste() const;
    /// Developer hook: saves the whole window, popups included, as a PNG.
    Q_INVOKABLE bool grabWindow(QQuickWindow *window, const QString &path);
};

extern "C" void maine_register_qml_types();
