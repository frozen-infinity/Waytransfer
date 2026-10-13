// MIT licensed regression fixture; exercises JetBrains Runtime, not the CLion product.
import java.awt.*;
import java.awt.event.*;
import java.nio.file.*;
import javax.swing.*;
import javax.swing.event.*;

public final class WaytransferSwingProbe {
    private static JPanel content(String marker, Color color) {
        JPanel panel = new JPanel(new BorderLayout());
        panel.setBackground(color);
        JTextField input = new JTextField("Type here");
        input.addFocusListener(new FocusAdapter() {
            @Override public void focusGained(FocusEvent event) {
                try { Files.writeString(Path.of(marker + "-focus"), "focused"); }
                catch (Exception error) { throw new RuntimeException(error); }
            }
            @Override public void focusLost(FocusEvent event) {
                try { Files.deleteIfExists(Path.of(marker + "-focus")); }
                catch (Exception error) { throw new RuntimeException(error); }
            }
        });
        input.addKeyListener(new KeyAdapter() {
            @Override public void keyPressed(KeyEvent event) {
                if (event.getKeyCode() == KeyEvent.VK_A || event.getKeyCode() == KeyEvent.VK_B) {
                    try { Files.writeString(Path.of(marker), event.getKeyChar() + ""); }
                    catch (Exception error) { throw new RuntimeException(error); }
                }
            }
        });
        input.getDocument().addDocumentListener(new DocumentListener() {
            private void changed() {
                try { Files.writeString(Path.of(marker + "-text"), input.getText()); }
                catch (Exception error) { throw new RuntimeException(error); }
            }
            public void insertUpdate(DocumentEvent event) { changed(); }
            public void removeUpdate(DocumentEvent event) { changed(); }
            public void changedUpdate(DocumentEvent event) { changed(); }
        });
        panel.add(input, BorderLayout.NORTH);
        panel.add(new JLabel("Waytransfer Java rendering test", SwingConstants.CENTER));
        return panel;
    }
    public static void main(String[] args) throws Exception {
        String toolkit = Toolkit.getDefaultToolkit().getClass().getName();
        System.out.println("Toolkit: " + toolkit);
        if (!toolkit.endsWith(args[0])) throw new AssertionError("Unexpected toolkit: " + toolkit);
        SwingUtilities.invokeAndWait(() -> {
            JFrame main = new JFrame("Waytransfer Java main");
            main.setUndecorated(true);
            main.addComponentListener(new ComponentAdapter() {
                @Override public void componentResized(ComponentEvent event) {
                    try { Files.writeString(Path.of("main-size"), main.getWidth() + "x" + main.getHeight()); }
                    catch (Exception error) { throw new RuntimeException(error); }
                }
            });
            main.setContentPane(content("main-key", Color.RED));
            main.setSize(640, 400);
            main.setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
            main.setVisible(true);
            System.out.println("Main shown");
            JDialog dialog = new JDialog(main, "Waytransfer Java dialog", false);
            dialog.setUndecorated(true);
            dialog.setContentPane(content("dialog-key", Color.BLUE));
            dialog.setSize(320, 220);
            dialog.setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
            dialog.setVisible(true);
            System.out.println("Dialog shown");
        });
    }
}
