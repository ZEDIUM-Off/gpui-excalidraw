use gpui_excalidraw::{
    Editor,
    core::{
        element::{Element, ElementBase, ElementType},
        scene::{AppState, Scene},
    },
    ui::editor::EmbeddedEvent,
};
use gpui_kit as gpui;
use gpui_kit::Modifiers;

fn rectangle(id: &str, x: f64) -> Element {
    let mut base = ElementBase::new(id.into(), ElementType::Rectangle);
    base.x = x;
    base.width = 100.0;
    base.height = 100.0;
    Element::Rectangle { base }
}
fn editor() -> Editor {
    let mut scene = Scene::new();
    scene.add(rectangle("a", 0.0));
    scene.add(rectangle("b", 300.0));
    Editor::new_embedded(scene, AppState::default())
}
#[test]
fn move_emits_transaction_not_samples_and_preserves_disjoint_delta() {
    let mut editor = editor();
    editor.on_canvas_mouse_down(50.0, 50.0, Modifiers::default(), 1);
    assert!(
        matches!(editor.take_embedded_events().as_slice(), [EmbeddedEvent::Begin { originals }] if originals.len() == 1 && originals[0].id() == "a")
    );
    editor.on_canvas_mouse_move(70.0, 80.0, Modifiers::default());
    assert!(editor.take_embedded_events().is_empty());
    editor
        .apply_embedded_delta(&[rectangle("b", 400.0)])
        .expect("disjoint update");
    editor.on_canvas_mouse_up(70.0, 80.0, Modifiers::default());
    assert!(
        matches!(editor.take_embedded_events().as_slice(), [EmbeddedEvent::Commit { originals, changes }] if originals.len() == 1 && changes[0].base().x == 20.0)
    );
    assert_eq!(editor.document.scene.get("b").expect("b").base().x, 400.0);
    assert!(editor.document.selected.contains("a"));
    assert!(!editor.history.can_undo());
}
#[test]
fn escape_restores_only_gesture_and_conflicting_delta_fails_atomically() {
    let mut editor = editor();
    editor.on_canvas_mouse_down(50.0, 50.0, Modifiers::default(), 1);
    editor.on_canvas_mouse_move(80.0, 50.0, Modifiers::default());
    assert!(
        editor
            .apply_embedded_delta(&[rectangle("b", 500.0), rectangle("a", 100.0)])
            .is_err()
    );
    assert_eq!(editor.document.scene.get("b").expect("b").base().x, 300.0);
    editor.handle_key("escape", None, Modifiers::default(), (800.0, 600.0));
    assert_eq!(editor.document.scene.get("a").expect("a").base().x, 0.0);
    assert!(matches!(
        editor.take_embedded_events().last(),
        Some(EmbeddedEvent::Cancel { .. })
    ));
}
#[test]
fn destructive_shortcuts_and_unsupported_tools_are_suppressed() {
    let mut editor = editor();
    editor.document.select("a");
    for key in [
        "delete",
        "backspace",
        "d",
        "x",
        "v",
        "z",
        "y",
        "s",
        "o",
        "n",
        "g",
    ] {
        editor.handle_key(
            key,
            None,
            Modifiers {
                control: true,
                ..Default::default()
            },
            (800.0, 600.0),
        );
    }
    editor.set_tool(gpui_excalidraw::ui::state::Tool::Rectangle);
    editor.on_canvas_mouse_down(600.0, 300.0, Modifiers::default(), 1);
    editor.on_canvas_mouse_up(650.0, 350.0, Modifiers::default());
    assert_eq!(editor.document.scene.len(), 2);
    assert!(!editor.history.can_undo());
    assert!(!editor.scene_open_pending());
    assert!(!editor.save_dialog_pending());
    assert!(
        editor
            .insert_image_bytes(b"invalid", "image/png", None)
            .is_none()
    );
}

#[test]
fn existing_text_commit_cancel_and_binding_dependency_closure() {
    use gpui_excalidraw::core::{
        element::{BindMode, FixedPointBinding},
        factory::{ElementOptions, new_arrow, new_text},
        geometry::Point,
    };
    let mut editor = editor();
    let mut label = new_text(10.0, 10.0, "old", 20.0, &ElementOptions::default());
    label.base_mut().id = "label".into();
    if let Element::Text(text) = &mut label {
        text.container_id = Some("a".into());
    }
    let mut arrow = new_arrow(
        vec![Point::new(100.0, 50.0), Point::new(300.0, 50.0)],
        None,
        None,
        &ElementOptions::default(),
    );
    arrow.base_mut().id = "arrow".into();
    if let Element::Arrow(arrow) = &mut arrow {
        arrow.linear.start_binding = Some(FixedPointBinding {
            element_id: "a".into(),
            fixed_point: [1.0, 0.5],
            mode: BindMode::Orbit,
        });
        arrow.linear.end_binding = Some(FixedPointBinding {
            element_id: "b".into(),
            fixed_point: [0.0, 0.5],
            mode: BindMode::Orbit,
        });
    }
    editor.document.scene.add(label);
    editor.document.scene.add(arrow);
    editor.on_canvas_mouse_down(50.0, 50.0, Modifiers::default(), 2);
    assert_eq!(editor.editing_text_id(), Some("label"));
    let events = editor.take_embedded_events();
    assert!(
        matches!(events.as_slice(), [EmbeddedEvent::Begin { originals }] if originals.len() == 4)
    );
    editor.handle_key("!", Some("!".into()), Modifiers::default(), (800.0, 600.0));
    editor.handle_key("enter", None, Modifiers::default(), (800.0, 600.0));
    assert!(
        matches!(editor.take_embedded_events().as_slice(), [EmbeddedEvent::Commit { changes, .. }] if changes.iter().any(|element| matches!(element, Element::Text(text) if text.text == "old!" && text.original_text == "old!")))
    );
    editor.on_canvas_mouse_down(50.0, 50.0, Modifiers::default(), 2);
    editor.handle_key("?", Some("?".into()), Modifiers::default(), (800.0, 600.0));
    editor.cancel_embedded_gesture();
    assert!(
        matches!(editor.document.scene.get("label"), Some(Element::Text(text)) if text.text == "old!")
    );
}

#[gpui_kit::test]
fn real_dispatch_emits_events_and_blur_cancels_without_focus_theft(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{AppContext, point, px, test::TestWindowExt};
    use std::{cell::RefCell, rc::Rc};
    let events = Rc::new(RefCell::new(Vec::new()));
    let (window, content, subscription) = cx.update(|cx| {
        gpui_kit::init(cx);
        let (window, content) =
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| editor())
            })
            .expect("window");
        let subscription = cx.subscribe(&content, {
            let events = events.clone();
            move |_, event: &EmbeddedEvent, _| events.borrow_mut().push(event.clone())
        });
        (window, content, subscription)
    });
    cx.update(|cx| {
        window
            .update(cx, |_, window, cx| {
                window.activate_window();
                window.render_frame(cx);
            })
            .expect("first frame")
    });
    cx.run_until_parked();
    cx.update(|cx| {
        window
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                window.drag(point(px(50.0), px(80.0)), point(px(70.0), px(100.0)), cx);
                assert_eq!(
                    content
                        .read(cx)
                        .document
                        .scene
                        .get("a")
                        .expect("a")
                        .base()
                        .x,
                    20.0
                );
                window.press("delete", cx);
                assert_eq!(content.read(cx).document.scene.len(), 2);
                content.update(cx, |editor, _| {
                    editor.on_canvas_mouse_down(70.0, 100.0, Modifiers::default(), 1);
                    editor.on_canvas_mouse_move(90.0, 110.0, Modifiers::default());
                });
            })
            .expect("drag update");
    });
    let other_focus = cx.update(|cx| cx.focus_handle());
    cx.update(|cx| {
        window
            .update(cx, |_, window, cx| {
                other_focus.focus(window, cx);
                window.render_frame(cx);
                assert!(other_focus.is_focused(window));
            })
            .expect("update");
    });
    cx.update(|cx| assert!(!content.read(cx).is_interacting()));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, EmbeddedEvent::Commit { .. }))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, EmbeddedEvent::Cancel { .. }))
    );
    drop(subscription);
}

#[test]
fn delta_preserves_viewport_and_selection_without_echo_and_pan_has_no_transaction() {
    use gpui_excalidraw::core::geometry::Point;
    let mut editor = editor();
    editor.document.zoom = 2.0;
    editor.document.scroll = Point::new(12.0, 34.0);
    editor.document.select("a");
    editor
        .apply_embedded_delta(&[rectangle("b", 450.0)])
        .expect("delta");
    assert!(editor.take_embedded_events().is_empty());
    assert_eq!(editor.document.zoom, 2.0);
    assert_eq!(editor.document.scroll, Point::new(12.0, 34.0));
    assert!(editor.document.selected.contains("a"));
    editor.set_tool(gpui_excalidraw::ui::state::Tool::Hand);
    editor.on_canvas_mouse_down(50.0, 50.0, Modifiers::default(), 1);
    editor.on_canvas_mouse_move(80.0, 90.0, Modifiers::default());
    editor.on_canvas_mouse_up(80.0, 90.0, Modifiers::default());
    assert_eq!(editor.document.scroll, Point::new(42.0, 74.0));
    assert!(editor.take_embedded_events().is_empty());
    assert!(!editor.history.can_undo());
}
