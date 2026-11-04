# Libraries ship their own consumer rules (Compose, Navigation, Coil, Media3, CameraX, kotlinx.serialization);
# :core's consumer-rules.pro keeps the UniFFI bindings and JNA.

# Navigation's type-safe routes are @Serializable classes.
-keepattributes RuntimeVisibleAnnotations,AnnotationDefault,InnerClasses,Signature
-keepclassmembers @kotlinx.serialization.Serializable class app.photovault.android.** {
    *** Companion;
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class app.photovault.android.**$$serializer { *; }

# ZXing has optional javax / java.awt paths that are never used on Android.
-dontwarn java.awt.**
-dontwarn javax.imageio.**
