package dev.junction;

import android.app.UiAutomation;
import android.content.Context;
import android.content.ContextWrapper;
import android.os.Build;
import android.os.Bundle;
import android.os.HandlerThread;
import android.os.Looper;
import android.util.Base64;
import android.view.accessibility.AccessibilityNodeInfo;

import java.io.BufferedReader;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;

/**
 * Types text into the focused text field of the device without the keyboard
 * or the clipboard: Running Devices starts it with
 * {@code CLASSPATH=<jar> app_process / dev.junction.TextInput} over adb shell
 * (the shell user may connect a UiAutomation, as {@code uiautomator} does)
 * and writes one base64 UTF-8 line per text. Each answer is one line:
 * {@code ok}, or {@code no <why>}. The first line is {@code ready} or
 * {@code fail <why>}. It exits when its input closes.
 */
public final class TextInput {
    /** UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES: TalkBack etc. keep running. */
    private static final int DONT_SUPPRESS_ACCESSIBILITY_SERVICES = 1;

    public static void main(String[] args) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, "UTF-8"));
        PrintStream out = new PrintStream(new FileOutputStream(FileDescriptor.out), true, "UTF-8");
        UiAutomation automation;
        try {
            automation = connect();
        } catch (Throwable t) {
            out.println("fail " + describe(t));
            System.exit(1);
            return;
        }
        out.println("ready");
        String line;
        while ((line = in.readLine()) != null) {
            line = line.trim();
            if (line.isEmpty()) {
                continue;
            }
            String answer;
            try {
                answer = insert(automation, new String(Base64.decode(line, Base64.DEFAULT), "UTF-8"));
            } catch (Throwable t) {
                answer = "no " + describe(t);
            }
            out.println(answer);
        }
        try {
            automation.disconnect();
        } catch (Throwable ignored) {
        }
        System.exit(0);
    }

    private static UiAutomation connect() throws Exception {
        HandlerThread thread = new HandlerThread("TextInput");
        thread.start();
        Looper looper = thread.getLooper();
        Object connection = Class.forName("android.app.UiAutomationConnection").getDeclaredConstructor().newInstance();
        Class<?> connectionType = Class.forName("android.app.IUiAutomationConnection");
        UiAutomation automation = null;
        for (Constructor<?> constructor : UiAutomation.class.getDeclaredConstructors()) {
            Class<?>[] types = constructor.getParameterTypes();
            constructor.setAccessible(true);
            if (types.length == 2 && types[0] == Looper.class && types[1] == connectionType) {
                automation = (UiAutomation) constructor.newInstance(looper, connection);
                break;
            }
        }
        if (automation == null) {
            Constructor<UiAutomation> constructor = UiAutomation.class.getDeclaredConstructor(Context.class, connectionType);
            constructor.setAccessible(true);
            automation = constructor.newInstance(new LooperContext(looper), connection);
        }
        try {
            Method connect = UiAutomation.class.getDeclaredMethod("connect", int.class);
            connect.setAccessible(true);
            connect.invoke(automation, DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
        } catch (NoSuchMethodException e) {
            Method connect = UiAutomation.class.getDeclaredMethod("connect");
            connect.setAccessible(true);
            connect.invoke(automation);
        }
        return automation;
    }

    /** Replaces the focused field's selection with {@code text}, caret after it. */
    private static String insert(UiAutomation automation, String text) {
        AccessibilityNodeInfo node = automation.findFocus(AccessibilityNodeInfo.FOCUS_INPUT);
        if (node == null) {
            AccessibilityNodeInfo root = automation.getRootInActiveWindow();
            node = root == null ? null : root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT);
        }
        if (node == null) {
            return "no No text field has the focus on the device.";
        }
        node.refresh();
        if (!node.isEditable()) {
            return "no The focused view on the device is not a text field.";
        }
        if (node.isPassword()) {
            return "no Password fields only take ASCII from the keyboard.";
        }
        CharSequence current = node.getText();
        if (current == null || (Build.VERSION.SDK_INT >= 26 && node.isShowingHintText())) {
            current = "";
        }
        int length = current.length();
        int start = node.getTextSelectionStart();
        int end = node.getTextSelectionEnd();
        if (start < 0 || end < 0 || start > length || end > length) {
            start = length;
            end = length;
        }
        if (start > end) {
            int swap = start;
            start = end;
            end = swap;
        }
        String value = current.subSequence(0, start) + text + current.subSequence(end, length);
        Bundle arguments = new Bundle();
        arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, value);
        if (!node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments)) {
            return "no The text field on the device did not take the text.";
        }
        int caret = start + text.length();
        Bundle selection = new Bundle();
        selection.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, caret);
        selection.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, caret);
        node.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selection);
        return "ok";
    }

    /** The exception the system service threw, in one line (a remote one's
     *  cause is only its stack trace). */
    private static String describe(Throwable t) {
        while (t instanceof java.lang.reflect.InvocationTargetException && t.getCause() != null) {
            t = t.getCause();
        }
        String message = t.getMessage();
        if (message != null) {
            message = message.split("\n", 2)[0].trim();
        }
        if (message != null && message.contains("already registered")) {
            return "Another UI automation (a running UI test, Appium, uiautomator) holds the device's accessibility connection.";
        }
        return t.getClass().getSimpleName() + (message == null || message.isEmpty() ? "" : ": " + message);
    }

    /** What UiAutomation(Context, ...) reads on Android versions without the Looper constructor. */
    private static final class LooperContext extends ContextWrapper {
        private final Looper looper;

        LooperContext(Looper looper) {
            super(null);
            this.looper = looper;
        }

        @Override
        public Looper getMainLooper() {
            return looper;
        }

        @Override
        public int getDisplayId() {
            return 0;
        }
    }
}
