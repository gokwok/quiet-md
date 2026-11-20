// 编辑器里的原生玻璃。每个浮层独立滚动；更新选项时保留正在输入的控件与拼音组合串。
#import <UIKit/UIKit.h>
#import <ImageIO/ImageIO.h>
extern void quiet_editor_action(const char *, size_t, const char *);
// 行号按 TextKit 的实际行片段定位，折行和滚动都跟随正文。
@interface QuietSourceView : UITextView
@end
@implementation QuietSourceView
- (void)drawRect:(CGRect)rect {
    [super drawRect:rect];
    NSLayoutManager *lm = self.layoutManager;
    NSRange visible = [lm glyphRangeForBoundingRect:CGRectOffset(rect,0,-self.textContainerInset.top) inTextContainer:self.textContainer];
    if (!visible.length && self.text.length) return;
    NSString *text = self.text;
    NSUInteger start = visible.length ? [lm characterIndexForGlyphAtIndex:visible.location] : 0;
    __block NSUInteger line = 1;
    [text enumerateSubstringsInRange:NSMakeRange(0,start) options:NSStringEnumerationByLines|NSStringEnumerationSubstringNotRequired usingBlock:^(__unused NSString *s, __unused NSRange a, __unused NSRange b, __unused BOOL *stop) { line++; }];
    if (start && [text characterAtIndex:start-1] != '\n') line--;
    NSDictionary *attrs = @{NSFontAttributeName:[UIFont monospacedSystemFontOfSize:13 weight:UIFontWeightRegular],NSForegroundColorAttributeName:UIColor.tertiaryLabelColor};
    [lm enumerateLineFragmentsForGlyphRange:visible usingBlock:^(CGRect r, __unused CGRect used, __unused NSTextContainer *c, NSRange range, __unused BOOL *stop) {
        NSUInteger i = [lm characterIndexForGlyphAtIndex:range.location];
        if (!i || [text characterAtIndex:i-1]=='\n') {
            [[NSString stringWithFormat:@"%lu",(unsigned long)line++] drawAtPoint:CGPointMake(8,r.origin.y+self.textContainerInset.top) withAttributes:attrs];
        }
    }];
    if (lm.extraLineFragmentTextContainer) [[NSString stringWithFormat:@"%lu",(unsigned long)line] drawAtPoint:CGPointMake(8,lm.extraLineFragmentRect.origin.y+self.textContainerInset.top) withAttributes:attrs];
}
@end

void quiet_hide_keyboard(void) {
    [UIApplication.sharedApplication sendAction:@selector(resignFirstResponder) to:nil from:nil forEvent:nil];
}
// 附件缩略图：后台按目标尺寸解码，缓存后再贴回仍指向同一路径的图片视图。
static NSCache<NSString *, UIImage *> *thumbs;
static UIColor *hexColor(NSString *hex) {
    unsigned v = 0; [[NSScanner scannerWithString:[hex stringByReplacingOccurrencesOfString:@"#" withString:@""]] scanHexInt:&v];
    return [UIColor colorWithRed:((v>>16)&255)/255.0 green:((v>>8)&255)/255.0 blue:(v&255)/255.0 alpha:1];
}
static void thumbnail(UIImageView *view, NSString *path, CGFloat pixels) {
    if (!thumbs) thumbs = [NSCache new];
    view.accessibilityIdentifier = path; UIImage *hit = [thumbs objectForKey:path];
    view.image = hit; if (hit) return;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED,0), ^{
        CGImageSourceRef src = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], NULL);
        CGImageRef cg = src ? CGImageSourceCreateThumbnailAtIndex(src,0,(__bridge CFDictionaryRef)@{(id)kCGImageSourceCreateThumbnailFromImageAlways:@YES,
            (id)kCGImageSourceCreateThumbnailWithTransform:@YES,(id)kCGImageSourceThumbnailMaxPixelSize:@(pixels)}) : NULL;
        UIImage *image = cg ? [UIImage imageWithCGImage:cg] : nil;
        if (cg) CGImageRelease(cg); if (src) CFRelease(src);
        dispatch_async(dispatch_get_main_queue(), ^{
            if (image) [thumbs setObject:image forKey:path];
            if ([view.accessibilityIdentifier isEqual:path]) view.image = image;
        });
    });
}
// 文档面板（目录 / 任务 / 附件 / 查找）是同一张从底部升起的纸：切换分段时沿用同一个视图，只有首次出现和关闭时动画。
static BOOL sheetKey(NSString *key) { return [key isEqual:@"find"] || [key isEqual:@"catalogue"]; }
@interface QuietEditorSurface : UIVisualEffectView <UITextFieldDelegate, UITextViewDelegate, UITableViewDataSource, UITableViewDelegate, UIGestureRecognizerDelegate>
@property NSString *key;
@property BOOL sheet;
@property CGFloat inset;
@property BOOL pulled;
@property BOOL fromList;
@property BOOL adopted;
@property NSInteger tab;
@property UIPanGestureRecognizer *pull;
@property UIControl *backdrop;
@property UIScrollView *scroll;
@property NSArray<NSDictionary *> *rows;
@property NSMutableDictionary<NSString *, UIView *> *fields;
@property NSMutableArray<UIView *> *lines;
@property NSString *model;
@property UITableView *list;
@property NSInteger listStart;
@property NSInteger activeRow;
@property NSDictionary *firstOption;
@end

@implementation QuietEditorSurface
- (void)send:(NSNumber *)action value:(NSString *)value {
    if (action) quiet_editor_action(self.key.UTF8String, action.unsignedIntegerValue, value.UTF8String);
}
- (void)tappedBackdrop {
    [self send:self.rows.firstObject[@"outside"] value:@""];
}
- (NSDictionary *)rowFor:(UIView *)v {
    return v.tag >= 0 && v.tag < (NSInteger)self.rows.count ? self.rows[v.tag] : nil;
}
- (void)changed:(UITextField *)field {
    if (!field.markedTextRange) [self send:[self rowFor:field][@"input"] value:field.text];
}
- (void)textViewDidChange:(UITextView *)field {
    if (!field.markedTextRange) [self send:[self rowFor:field][@"input"] value:field.text];
}
- (BOOL)textView:(UITextView *)field shouldChangeTextInRange:(NSRange)range replacementText:(NSString *)text {
    if (![self.key isEqual:@"equation"]) return YES;
    NSString *old = field.text;
    if (NSMaxRange(range) > old.length) return NO;
    NSString *kept = [old stringByReplacingCharactersInRange:range withString:@""];
    NSUInteger count = [kept lengthOfBytesUsingEncoding:NSUTF32LittleEndianStringEncoding]/4;
    NSUInteger room = count < 10000 ? 10000-count : 0;
    NSData *replacement = [text dataUsingEncoding:NSUTF32LittleEndianStringEncoding];
    if (replacement.length/4 <= room) return YES;
    NSString *prefix = [[NSString alloc] initWithBytes:replacement.bytes length:room*4 encoding:NSUTF32LittleEndianStringEncoding];
    [field.textStorage replaceCharactersInRange:range withString:prefix];
    field.selectedRange = NSMakeRange(range.location+prefix.length,0);
    [self textViewDidChange:field]; return NO;
}
- (BOOL)textFieldShouldReturn:(UITextField *)field {
    [self send:[self rowFor:field][@"submit"] value:@""]; return NO;
}
- (UIView *)input:(NSDictionary *)r index:(NSInteger)i {
    NSString *key = r[@"field"];
    UIView *view = self.fields[key];
    if (!view) {
        if ([r[@"multi"] boolValue]) {
            UITextView *t = [[QuietSourceView alloc] initWithFrame:CGRectZero textContainer:nil]; t.textContainerInset = UIEdgeInsetsMake(8,36,12,10); t.delegate = self;
            t.font = [UIFont monospacedSystemFontOfSize:16 weight:UIFontWeightRegular]; t.text = r[@"value"]; view = t;
        } else {
            UITextField *t = [UITextField new]; t.delegate = self; t.placeholder = key; t.text = r[@"value"];
            t.font = [UIFont preferredFontForTextStyle:UIFontTextStyleBody];
            t.leftView = [[UIView alloc] initWithFrame:CGRectMake(0,0,12,1)]; t.leftViewMode = UITextFieldViewModeAlways;
            if ([self.key isEqual:@"find"]) {
                UIImageView *glyph=[[UIImageView alloc] initWithImage:[UIImage systemImageNamed:[r[@"search"] boolValue] ? @"magnifyingglass" : @"arrow.turn.down.right"]];
                glyph.tintColor=UIColor.secondaryLabelColor; glyph.contentMode=UIViewContentModeCenter; glyph.frame=CGRectMake(4,0,32,36);
                UIView *lead=[[UIView alloc] initWithFrame:CGRectMake(0,0,38,36)]; [lead addSubview:glyph];
                t.leftView=lead; t.clearButtonMode=UITextFieldViewModeWhileEditing;
            }
            [t addTarget:self action:@selector(changed:) forControlEvents:UIControlEventEditingChanged]; view = t;
        }
        view.accessibilityLabel = key;
        if ([r[@"multi"] boolValue]) view.accessibilityIdentifier=[@"quiet.field." stringByAppendingString:self.key];
        if ([self.key isEqual:@"link"] || [self.key isEqual:@"equation"] || [self.key isEqual:@"language"]) {
            id<UITextInputTraits> input = (id)view;
            input.autocapitalizationType = UITextAutocapitalizationTypeNone; input.autocorrectionType = UITextAutocorrectionTypeNo;
            input.smartQuotesType = UITextSmartQuotesTypeNo; input.smartDashesType = UITextSmartDashesTypeNo;
        }
        view.backgroundColor = [r[@"multi"] boolValue] ? UIColor.secondarySystemBackgroundColor : UIColor.secondarySystemFillColor; view.layer.cornerRadius = 12;
        self.fields[key] = view; [self.scroll addSubview:view];
        if ([r[@"focus"] boolValue]) { [view becomeFirstResponder];
            if ([self.key isEqual:@"equation"] || [self.key isEqual:@"find"]) [(UIResponder *)view selectAll:nil]; }
    }
    view.tag = i; return view;
}
- (UIView<UITextInput> *)focusedInput {
    for (UIView *v in self.fields.allValues) if (v.isFirstResponder) return (id)v;
    return nil;
}
- (NSArray<UIKeyCommand *> *)keyCommands {
    UIView<UITextInput> *field = [self focusedInput];
    if (!field || field.markedTextRange) return @[];
    NSMutableArray *keys = [NSMutableArray arrayWithObject:UIKeyInputEscape];
    if ([self.key isEqual:@"language"]) [keys addObjectsFromArray:@[UIKeyInputUpArrow,UIKeyInputDownArrow]];
    if ([self.key isEqual:@"find"]) [keys addObject:@"\r"];
    if ([self.key isEqual:@"equation"] && field.selectedTextRange.empty) {
        if ([field comparePosition:field.selectedTextRange.start toPosition:field.beginningOfDocument] == NSOrderedSame) [keys addObject:UIKeyInputLeftArrow];
        if ([field comparePosition:field.selectedTextRange.end toPosition:field.endOfDocument] == NSOrderedSame) [keys addObject:UIKeyInputRightArrow];
    }
    NSMutableArray *commands = [NSMutableArray array];
    for (NSString *key in keys) {
        UIKeyCommand *c = [UIKeyCommand keyCommandWithInput:key modifierFlags:[key isEqual:@"\r"] ? UIKeyModifierShift : 0 action:@selector(fieldCommand:)];
        c.wantsPriorityOverSystemBehavior = YES; [commands addObject:c];
    }
    return commands;
}
- (void)fieldCommand:(UIKeyCommand *)command {
    NSDictionary *names = @{UIKeyInputEscape:@"escape",UIKeyInputUpArrow:@"up",UIKeyInputDownArrow:@"down",
        UIKeyInputLeftArrow:@"leftEdge",UIKeyInputRightArrow:@"rightEdge",@"\r":@"previous"};
    [self send:[self rowFor:[self focusedInput]][@"submit"] value:names[command.input]];
}
- (UIAction *)action:(NSDictionary *)item owner:(UIView *)owner {
    __weak QuietEditorSurface *weak = self;
    __weak UIView *source = owner;
    UIImage *image = [item[@"symbol"] length] ? [UIImage systemImageNamed:item[@"symbol"]] : nil;
    return [UIAction actionWithTitle:item[@"title"] image:image identifier:nil handler:^(__unused UIAction *a) {
        if (!weak || ![source isDescendantOfView:weak]) return;
        UIView<UITextInput> *field = [weak focusedInput];
        if (field.markedTextRange) {
            [field unmarkText];
            if ([field isKindOfClass:UITextField.class]) [weak changed:(id)field];
            else [weak textViewDidChange:(id)field];
        }
        [weak send:item[@"action"] value:@""];
    }];
}
- (UIMenu *)menu:(NSArray *)children owner:(UIView *)owner {
    // 同一 group 的选项放进一段，段与段之间是系统分隔线。
    NSMutableArray *sections = [NSMutableArray array], *actions = nil; id group = nil;
    for (NSDictionary *child in children) {
        if (!actions || ![child[@"group"] isEqual:group]) { actions = [NSMutableArray array]; [sections addObject:actions]; group = child[@"group"]; }
        UIAction *a = [self action:child owner:owner]; if ([child[@"disabled"] boolValue]) a.attributes=UIMenuElementAttributesDisabled;
        a.state = [child[@"on"] boolValue] ? UIMenuElementStateOn : UIMenuElementStateOff; [actions addObject:a];
    }
    if (sections.count < 2) return [UIMenu menuWithChildren:sections.firstObject ?: @[]];
    NSMutableArray *menus = [NSMutableArray array];
    for (NSArray *part in sections) [menus addObject:[UIMenu menuWithTitle:@"" image:nil identifier:nil options:UIMenuOptionsDisplayInline children:part]];
    return [UIMenu menuWithChildren:menus];
}
- (UIView *)line:(NSArray *)items width:(CGFloat)width height:(CGFloat)height {
    UIScrollView *strip = [UIScrollView new]; strip.showsHorizontalScrollIndicator = NO;
    CGFloat x = 0;
    for (NSDictionary *item in items) {
        UIButtonConfiguration *c = UIButtonConfiguration.plainButtonConfiguration;
        NSString *symbol = item[@"symbol"], *title = item[@"title"];
        BOOL iconOnly = [item[@"iconOnly"] boolValue], single = items.count == 1, outline = [item[@"outline"] boolValue], on = [item[@"on"] boolValue];
        c.title = iconOnly ? nil : title;
        if ([item[@"label"] boolValue]) {
            UILabel *label = [UILabel new]; label.text = title; label.textColor=UIColor.secondaryLabelColor; label.font=[UIFont preferredFontForTextStyle:self.sheet ? UIFontTextStyleSubheadline : UIFontTextStyleFootnote];
            CGFloat span=[label sizeThatFits:CGSizeMake(width,height)].width+8; label.frame=CGRectMake(x+(self.sheet ? 8 : 4),0,span,height); [strip addSubview:label]; x+=span; continue;
        }
        if (symbol.length) c.image = [UIImage systemImageNamed:symbol];
        if ([item[@"match"] count]==2) {
            NSMutableAttributedString *snippet = [[NSMutableAttributedString alloc] initWithString:title attributes:@{NSFontAttributeName:[UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline]}];
            NSRange range = NSMakeRange([item[@"match"][0] unsignedIntegerValue],[item[@"match"][1] unsignedIntegerValue]);
            if (NSMaxRange(range)<=title.length) [snippet addAttribute:NSFontAttributeName value:[UIFont boldSystemFontOfSize:15] range:range];
            c.attributedTitle = snippet; c.titleLineBreakMode = NSLineBreakByTruncatingTail; c.background.backgroundColor = UIColor.secondarySystemBackgroundColor;
        }
        c.imagePadding = 8; c.cornerStyle = UIButtonConfigurationCornerStyleCapsule;
        c.contentInsets = NSDirectionalEdgeInsetsMake(0, 8, 0, 8);
        if ([item[@"stack"] boolValue]) { c.imagePlacement = NSDirectionalRectEdgeTop; c.imagePadding = 8; }
        if (on && !outline) c.background.backgroundColor = UIColor.tertiarySystemFillColor;
        c.baseForegroundColor = [item[@"danger"] boolValue] ? UIColor.systemRedColor : (on && !outline ? self.tintColor : UIColor.labelColor);
        if (outline) {
            // 目录、任务、附件行：正文字号，当前项加粗变深（底色由行本身画），其余略浅。
            UIFont *body = [UIFont preferredFontForTextStyle:UIFontTextStyleBody];
            UIFont *font = on ? [UIFont systemFontOfSize:body.pointSize weight:UIFontWeightSemibold] : body;
            c.titleTextAttributesTransformer = ^NSDictionary *(NSDictionary *attrs) { NSMutableDictionary *m = attrs.mutableCopy; m[NSFontAttributeName] = font; return m; };
            c.baseForegroundColor = on ? UIColor.labelColor : ([item[@"done"] boolValue] ? UIColor.tertiaryLabelColor : UIColor.secondaryLabelColor);
            UIColor *glyph = [item[@"done"] boolValue] ? self.tintColor : UIColor.tertiaryLabelColor;
            c.imageColorTransformer = ^UIColor *(__unused UIColor *color) { return glyph; };
            c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:16 weight:UIImageSymbolWeightRegular];
            c.contentInsets = NSDirectionalEdgeInsetsMake(0, 12, 0, 8); c.imagePadding = 10;
        }
        if ([item[@"minor"] boolValue]) {
            c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:13 weight:UIImageSymbolWeightSemibold];
            c.baseForegroundColor = UIColor.tertiaryLabelColor;
        }
        if ([item[@"muted"] boolValue]) c.baseForegroundColor = UIColor.secondaryLabelColor;
        if ([item[@"menuTitle"] boolValue]) {
            // 「查找 ⌄」：文字在前，小号下箭头跟在后面。
            c.imagePlacement = NSDirectionalRectEdgeTrailing; c.imagePadding = 4; c.baseForegroundColor = UIColor.secondaryLabelColor;
            c.preferredSymbolConfigurationForImage = [UIImageSymbolConfiguration configurationWithPointSize:11 weight:UIImageSymbolWeightSemibold];
            UIFont *font = [UIFont systemFontOfSize:[UIFont preferredFontForTextStyle:UIFontTextStyleBody].pointSize weight:UIFontWeightMedium];
            c.titleTextAttributesTransformer = ^NSDictionary *(NSDictionary *attrs) { NSMutableDictionary *m = attrs.mutableCopy; m[NSFontAttributeName] = font; return m; };
        }
        UIButton *b = [UIButton buttonWithConfiguration:c primaryAction:nil];
        if (item[@"swatch"]) {
            NSArray *rgba = item[@"swatch"];
            if (rgba.count == 4) {
                UIColor *color = [UIColor colorWithRed:[rgba[0] doubleValue] green:[rgba[1] doubleValue] blue:[rgba[2] doubleValue] alpha:[rgba[3] doubleValue]];
                c.image = [[UIImage systemImageNamed:@"circle.fill"] imageWithTintColor:color renderingMode:UIImageRenderingModeAlwaysOriginal];
            } else c.image = [UIImage systemImageNamed:@"circle.slash"];
            b.configuration = c;
        }
        b.accessibilityIdentifier = [NSString stringWithFormat:@"quiet.%@.%@",self.key,symbol.length ? symbol : title];
        b.accessibilityLabel = title; b.enabled = ![item[@"disabled"] boolValue];
        if (on) b.accessibilityTraits |= UIAccessibilityTraitSelected;
        if ([item[@"children"] count]) {
            b.menu = [self menu:item[@"children"] owner:b]; b.showsMenuAsPrimaryAction = YES;
        } else [b addAction:[self action:item owner:b] forControlEvents:UIControlEventTouchUpInside];
        CGFloat span = single ? width : MAX(44, iconOnly ? 44 : [b sizeThatFits:CGSizeMake(width,height)].width);
        if ([item[@"equal"] boolValue]) span = width/items.count;
        if ([item[@"trailing"] boolValue]) { if (single) span=44; x = MAX(x,width-span-[item[@"reserve"] doubleValue]); }
        if ([item[@"fill"] boolValue]) span = MAX(44,width-x-[item[@"reserve"] doubleValue]);
        b.titleLabel.lineBreakMode = NSLineBreakByTruncatingTail;
        if (single || [item[@"fill"] boolValue]) b.contentHorizontalAlignment = UIControlContentHorizontalAlignmentLeading;
        if ([self.key isEqual:@"selection"] && ( [symbol isEqual:@"plus.circle"] || [symbol isEqual:@"function"] )) b.contentHorizontalAlignment = UIControlContentHorizontalAlignmentCenter;
        b.frame = CGRectMake(x,0,span,[item[@"stack"] boolValue] ? 80 : height); [strip addSubview:b];
        x += span;
    }
    strip.contentSize = CGSizeMake(x,height); strip.scrollEnabled = x > width; return strip;
}
- (UIView *)documentHeader:(NSDictionary *)r width:(CGFloat)w height:(CGFloat)h {
    // 左边是这篇文档的小缩略图（和主页列表行首同一套，Rust 的 `mini_page` 算好每道线），有自己的图标时放图标；标题下一行写它所在的空间，右边灰色圆形关闭钮。
    UIView *header = [UIView new]; CGFloat mid = h/2;
    NSDictionary *t = r[@"thumb"]; NSString *emoji = [t[@"icon"] isKindOfClass:NSString.class] ? t[@"icon"] : nil;
    if (emoji.length) {
        UILabel *icon = [UILabel new]; icon.text = emoji; icon.font = [UIFont systemFontOfSize:26]; icon.textAlignment = NSTextAlignmentCenter;
        icon.frame = CGRectMake(4,mid-18,34,36); [header addSubview:icon];
    } else if (t) {
        CGFloat pw = [t[@"w"] doubleValue], ph = [t[@"h"] doubleValue];
        UIColor *ink = hexColor(t[@"ink"]);
        UIView *page = [[UIView alloc] initWithFrame:CGRectMake(21-pw/2,mid-ph/2,pw,ph)];
        page.backgroundColor = hexColor(t[@"paper"]); page.layer.cornerRadius = 4; page.layer.cornerCurve = kCACornerCurveContinuous; page.clipsToBounds = YES;
        page.layer.borderWidth = 0.5; page.layer.borderColor = UIColor.separatorColor.CGColor;
        for (NSDictionary *q in t[@"parts"]) {
            CGRect f = CGRectMake([q[@"x"] doubleValue],[q[@"y"] doubleValue],[q[@"w"] doubleValue],[q[@"h"] doubleValue]);
            UIColor *c = [(q[@"c"] ? hexColor(q[@"c"]) : ink) colorWithAlphaComponent:[q[@"a"] doubleValue]];
            UIView *v;
            if (q[@"img"]) { UIImageView *iv = [[UIImageView alloc] initWithFrame:f]; iv.contentMode = UIViewContentModeScaleAspectFill; iv.backgroundColor = c; thumbnail(iv,q[@"img"],MAX(f.size.width,f.size.height)*4); v = iv; }
            else { v = [[UIView alloc] initWithFrame:f]; if (q[@"stroke"]) { v.layer.borderWidth = [q[@"stroke"] doubleValue]; v.layer.borderColor = c.CGColor; } else v.backgroundColor = c; }
            v.layer.cornerRadius = [q[@"r"] doubleValue]; v.clipsToBounds = YES; [page addSubview:v];
        }
        [header addSubview:page];
    }
    UILabel *label = [UILabel new]; label.text = r[@"header"]; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
    label.frame = CGRectMake(48,mid-21,w-100,22); [header addSubview:label];
    UILabel *space = [UILabel new]; space.text = r[@"space"]; space.font = [UIFont preferredFontForTextStyle:UIFontTextStyleFootnote];
    space.textColor = UIColor.secondaryLabelColor; space.frame = CGRectMake(48,mid+2,w-100,18); [header addSubview:space];
    NSDictionary *item = [r[@"items"] firstObject];
    UIButton *close = [UIButton buttonWithType:UIButtonTypeSystem];
    UIImageSymbolConfiguration *look = [[UIImageSymbolConfiguration configurationWithPointSize:28] configurationByApplyingConfiguration:
        [UIImageSymbolConfiguration configurationWithPaletteColors:@[UIColor.secondaryLabelColor,UIColor.tertiarySystemFillColor]]];
    [close setImage:[UIImage systemImageNamed:@"xmark.circle.fill" withConfiguration:look] forState:UIControlStateNormal];
    close.accessibilityIdentifier = [@"quiet.close." stringByAppendingString:self.key]; close.accessibilityLabel = item[@"title"];
    close.frame = CGRectMake(w-44,mid-22,44,44); [header addSubview:close];
    [close addAction:[self action:item owner:close] forControlEvents:UIControlEventTouchUpInside];
    return header;
}
- (void)update:(NSArray *)rows width:(CGFloat)w {
    NSArray *old = self.rows; self.rows = rows;
    CGFloat y = 6, x = self.inset;
    NSUInteger boundary = [rows indexOfObjectPassingTest:^BOOL(NSDictionary *r, __unused NSUInteger idx, __unused BOOL *stop) { return [r[@"list"] boolValue]; }];
    BOOL listed = boundary != NSNotFound, switched = NO;
    self.listStart = listed ? boundary+1 : rows.count;
    NSMutableSet *keep = [NSMutableSet set]; NSMutableArray *fresh = [NSMutableArray array]; BOOL below = NO;
    for (NSInteger i = 0; i < self.listStart; ++i) {
        NSDictionary *r = rows[i];
        CGFloat h = r[@"h"] ? [r[@"h"] doubleValue] : (r[@"caption"] ? 28 : ([r[@"multi"] boolValue] ? 96 : 44));
        if ([r[@"stretch"] boolValue] || [r[@"multi"] boolValue]) h = MAX(44,self.bounds.size.height-y-54);
        UIView *v, *prior = i < (NSInteger)self.lines.count ? self.lines[i] : nil;
        if (r[@"field"]) {
            v = [self input:r index:i]; [keep addObject:r[@"field"]];
            if (r[@"tokens"] && [v isKindOfClass:UITextView.class] && !(i<(NSInteger)old.count && [old[i] isEqual:r])) {
                UITextView *code=(id)v;
                if (!code.markedTextRange && [code.text isEqual:r[@"value"]]) {
                    NSTextStorage *storage=code.textStorage; [storage beginEditing];
                    [storage addAttribute:NSForegroundColorAttributeName value:UIColor.labelColor range:NSMakeRange(0,code.text.length)];
                    for (NSArray *t in r[@"tokens"]) {
                        UIColor *color=[UIColor colorWithRed:[t[2] doubleValue] green:[t[3] doubleValue] blue:[t[4] doubleValue] alpha:1];
                        [storage addAttribute:NSForegroundColorAttributeName value:color range:NSMakeRange([t[0] unsignedIntegerValue],[t[1] unsignedIntegerValue])];
                    }
                    [storage endEditing];
                }
            }
        }
        else if (i < (NSInteger)old.count && [old[i] isEqual:r] && prior.frame.size.width == w) v = prior;
        else if (r[@"header"] && r[@"space"]) v = [self documentHeader:r width:w height:h];
        else if (r[@"header"]) {
            UIView *header = [UIView new]; BOOL source = [r[@"source"] boolValue];
            UILabel *label = [UILabel new]; label.text = r[@"header"]; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
            label.frame = CGRectMake(source ? 48 : 4,0,w-(source ? 96 : 52),h); label.textAlignment = source ? NSTextAlignmentCenter : NSTextAlignmentLeft;
            [header addSubview:label]; NSDictionary *item = [r[@"items"] firstObject];
            UIButton *close = [UIButton buttonWithType:UIButtonTypeSystem]; [close setImage:[UIImage systemImageNamed:source ? @"checkmark.circle.fill" : @"xmark.circle.fill"] forState:UIControlStateNormal];
            close.tintColor = source ? self.tintColor : UIColor.tertiaryLabelColor;
            [close setPreferredSymbolConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:source ? 33 : 23] forImageInState:UIControlStateNormal];
            close.accessibilityIdentifier = [@"quiet.close." stringByAppendingString:self.key];
            close.frame = CGRectMake(w-44,(h-44)/2,44,44); close.accessibilityLabel = item[@"title"];
            [header addSubview:close]; [close addAction:[self action:item owner:close] forControlEvents:UIControlEventTouchUpInside]; v = header;
        } else if ([r[@"style"] isEqual:@"tabs"]) {
            // 换分段时沿用原来的分段控件，iOS 26 的选中镜片才能一路滑过去。
            NSArray *items = r[@"items"]; UISegmentedControl *tabs = [prior isKindOfClass:UISegmentedControl.class] && ((UISegmentedControl *)prior).numberOfSegments == items.count ? (id)prior : nil;
            BOOL reuse = tabs != nil; if (!reuse) tabs = [UISegmentedControl new];
            for (NSUInteger j=0;j<items.count;j++) {
                NSDictionary *item = items[j]; UIAction *action = [self action:item owner:tabs];
                if (reuse) [tabs setAction:action forSegmentAtIndex:j]; else [tabs insertSegmentWithAction:action atIndex:j animated:NO];
                if (![item[@"on"] boolValue]) continue;
                if (tabs.selectedSegmentIndex != (NSInteger)j) tabs.selectedSegmentIndex = j;
                switched = reuse && self.tab != (NSInteger)j; self.tab = j;
            }
            v = tabs;
        } else if (r[@"caption"]) {
            UILabel *label = [UILabel new]; label.text = r[@"caption"]; label.textColor = UIColor.secondaryLabelColor;
            label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleFootnote]; v = label;
        } else v = [self line:r[@"items"] width:w height:h];
        if (prior) {
            if (prior != v && ![self.fields.allValues containsObject:prior]) {
                if ([prior isKindOfClass:UIScrollView.class] && [v isKindOfClass:UIScrollView.class])
                    ((UIScrollView *)v).contentOffset = ((UIScrollView *)prior).contentOffset;
                [prior removeFromSuperview];
            }
            self.lines[i] = v;
        } else [self.lines addObject:v];
        if (!v.superview) { [self.scroll addSubview:v]; if (below) [fresh addObject:v]; }
        if ([r[@"style"] isEqual:@"tabs"]) below = YES;
        if ([r[@"multi"] boolValue]) {
            v.layer.cornerRadius=0; v.frame=CGRectMake(0,y,self.bounds.size.width,h);
        } else if (r[@"field"] && self.sheet) v.frame=CGRectMake(x,y+3,w,h-6);
        else if ([r[@"style"] isEqual:@"tabs"] && self.sheet) v.frame=CGRectMake(x,y+2,w,h-12);
        else if (r[@"caption"] && self.sheet) v.frame=CGRectMake(x+8,y,w-8,h);
        else v.frame = CGRectMake(x,y,w,h);
        y += h;
    }
    while ((NSInteger)self.lines.count > self.listStart) {
        UIView *v = self.lines.lastObject;
        if (![self.fields.allValues containsObject:v]) [v removeFromSuperview]; [self.lines removeLastObject];
    }
    for (NSString *key in self.fields.allKeys) if (![keep containsObject:key]) {
        BOOL focused = self.fields[key].isFirstResponder;
        [self.fields[key] removeFromSuperview]; [self.fields removeObjectForKey:key];
        if (focused && keep.count) [[self.fields objectForKey:keep.anyObject] becomeFirstResponder];
    }
    if (self.sheet && (switched || self.adopted) && old) {
        // 换分段：分段控件以下的新内容淡入，列表交叉淡变。
        for (UIView *v in fresh) { v.alpha = 0; [UIView animateWithDuration:0.22 animations:^{ v.alpha = 1; }]; }
        if (self.list) { [UIView transitionWithView:self.list duration:0.22 options:UIViewAnimationOptionTransitionCrossDissolve|UIViewAnimationOptionAllowUserInteraction animations:nil completion:nil]; self.list.contentOffset = CGPointZero; self.activeRow = NSNotFound; }
    }
    self.adopted = NO;
    if (listed) {
        if (!self.list) {
            self.list = [[UITableView alloc] initWithFrame:CGRectZero style:UITableViewStylePlain];
            self.list.dataSource = self; self.list.delegate = self;
            self.list.separatorStyle = UITableViewCellSeparatorStyleNone; self.list.backgroundColor = UIColor.clearColor;
            self.list.allowsSelection = NO; self.list.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
            self.list.estimatedRowHeight = 0;
            [self.scroll addSubview:self.list]; self.activeRow = NSNotFound;
        }
        self.list.frame = CGRectMake(x,y,w,MAX(0,self.bounds.size.height-y-6));
        self.list.contentInset=UIEdgeInsetsMake(0,0,self.safeAreaInsets.bottom,0); [self.list reloadData];
        NSInteger active = NSNotFound;
        for (NSInteger i = self.listStart; i < (NSInteger)rows.count; ++i) {
            if ([rows[i][@"on"] boolValue] || [[rows[i][@"items"] firstObject][@"on"] boolValue]) { active = i-self.listStart; break; }
        }
        NSDictionary *first = self.listStart < (NSInteger)rows.count ? rows[self.listStart] : nil;
        if (active != NSNotFound && active != self.activeRow)
            [self.list scrollToRowAtIndexPath:[NSIndexPath indexPathForRow:active inSection:0] atScrollPosition:UITableViewScrollPositionNone animated:NO];
        else if (![self.key isEqual:@"catalogue"] && ![first isEqual:self.firstOption]) self.list.contentOffset = CGPointZero;
        self.activeRow = active; self.firstOption = first; self.scroll.contentSize = self.bounds.size;
    } else self.scroll.contentSize = CGSizeMake(self.bounds.size.width,y+6);
}
- (NSInteger)tableView:(UITableView *)table numberOfRowsInSection:(NSInteger)section {
    return (NSInteger)self.rows.count-self.listStart;
}
- (CGFloat)tableView:(UITableView *)table heightForRowAtIndexPath:(NSIndexPath *)path {
    NSDictionary *row = self.rows[self.listStart + path.row];
    if (row[@"empty"]) return MAX(200,table.bounds.size.height*0.62);
    if (row[@"media"]) return floor((table.bounds.size.width-10)/2*0.75)+10;
    if (row[@"snippet"]) return ceil([[self snippet:row] sizeThatFits:CGSizeMake(table.bounds.size.width-48,CGFLOAT_MAX)].height)+26;
    return row[@"h"] ? [row[@"h"] doubleValue] : 44;
}
- (UILabel *)snippet:(NSDictionary *)row {
    NSString *text = row[@"snippet"]; UIFont *font = [UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline];
    NSMutableAttributedString *body = [[NSMutableAttributedString alloc] initWithString:text attributes:@{NSFontAttributeName:font,NSForegroundColorAttributeName:UIColor.secondaryLabelColor}];
    NSArray *match = row[@"match"]; NSRange range = match.count==2 ? NSMakeRange([match[0] unsignedIntegerValue],[match[1] unsignedIntegerValue]) : NSMakeRange(0,0);
    if (range.length && NSMaxRange(range)<=text.length) [body addAttributes:@{NSFontAttributeName:[UIFont systemFontOfSize:font.pointSize weight:UIFontWeightSemibold],NSForegroundColorAttributeName:UIColor.labelColor} range:range];
    UILabel *label = [UILabel new]; label.attributedText = body; label.numberOfLines = 2; label.lineBreakMode = NSLineBreakByTruncatingTail; label.userInteractionEnabled = NO;
    return label;
}
- (UIButton *)card:(NSDictionary *)row frame:(CGRect)frame {
    // 查找结果：浅灰圆角卡片，段落图标对齐第一行，最多两行，命中处加粗变深。
    UIButton *card = [UIButton buttonWithType:UIButtonTypeCustom]; card.frame = frame;
    card.backgroundColor = [row[@"on"] boolValue] ? UIColor.tertiarySystemFillColor : UIColor.secondarySystemBackgroundColor;
    card.layer.cornerRadius = 12; card.layer.cornerCurve = kCACornerCurveContinuous;
    UILabel *label = [self snippet:row]; label.frame = CGRectMake(36,10,frame.size.width-48,frame.size.height-20); [card addSubview:label];
    UIImageView *glyph = [[UIImageView alloc] initWithImage:[UIImage systemImageNamed:row[@"symbol"] ?: @"text.alignleft" withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:13 weight:UIImageSymbolWeightMedium]]];
    glyph.tintColor = UIColor.tertiaryLabelColor; glyph.contentMode = UIViewContentModeCenter; glyph.userInteractionEnabled = NO;
    glyph.frame = CGRectMake(10,10,20,[UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline].lineHeight); [card addSubview:glyph];
    card.accessibilityLabel = row[@"snippet"]; if ([row[@"on"] boolValue]) card.accessibilityTraits |= UIAccessibilityTraitSelected;
    [card addAction:[self action:row owner:card] forControlEvents:UIControlEventTouchUpInside];
    [card addAction:[UIAction actionWithHandler:^(UIAction *a) { ((UIView *)a.sender).alpha = 0.55; }] forControlEvents:UIControlEventTouchDown|UIControlEventTouchDragEnter];
    [card addAction:[UIAction actionWithHandler:^(UIAction *a) { [UIView animateWithDuration:0.2 animations:^{ ((UIView *)a.sender).alpha = 1; }]; }] forControlEvents:UIControlEventTouchUpInside|UIControlEventTouchUpOutside|UIControlEventTouchCancel|UIControlEventTouchDragExit];
    return card;
}
- (UIView *)tile:(NSDictionary *)item frame:(CGRect)frame {
    UIButton *tile = [UIButton buttonWithType:UIButtonTypeCustom]; tile.frame = frame;
    tile.backgroundColor = UIColor.secondarySystemBackgroundColor; tile.layer.cornerRadius = 12; tile.layer.cornerCurve = kCACornerCurveContinuous; tile.clipsToBounds = YES;
    UIImageView *image = [[UIImageView alloc] initWithFrame:tile.bounds]; image.contentMode = UIViewContentModeScaleAspectFill; image.userInteractionEnabled = NO;
    NSString *path = item[@"path"];
    if (path.length) thumbnail(image,path,MAX(frame.size.width,frame.size.height)*self.traitCollection.displayScale);
    else { image.image = [UIImage systemImageNamed:@"photo" withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:22]]; image.tintColor = UIColor.tertiaryLabelColor; image.contentMode = UIViewContentModeCenter; }
    [tile addSubview:image]; tile.accessibilityLabel = item[@"title"];
    [tile addAction:[self action:item owner:tile] forControlEvents:UIControlEventTouchUpInside];
    return tile;
}
- (UITableViewCell *)tableView:(UITableView *)table cellForRowAtIndexPath:(NSIndexPath *)path {
    UITableViewCell *cell = [table dequeueReusableCellWithIdentifier:@"option"];
    if (!cell) cell = [[UITableViewCell alloc] initWithStyle:UITableViewCellStyleDefault reuseIdentifier:@"option"];
    cell.backgroundColor = UIColor.clearColor;
    for (UIView *old in cell.contentView.subviews.copy) if (old.tag == 101) [old removeFromSuperview];
    NSDictionary *row = self.rows[self.listStart + path.row];
    CGFloat W = table.bounds.size.width, h = [self tableView:table heightForRowAtIndexPath:path];
    UIView *v;
    if (row[@"caption"]) {
        UILabel *label = [UILabel new]; label.text = row[@"caption"]; label.textColor = UIColor.secondaryLabelColor;
        label.font = [UIFont preferredFontForTextStyle:self.sheet ? UIFontTextStyleFootnote : UIFontTextStyleBody];
        label.frame = CGRectMake(self.sheet ? 8 : 16,0,W-16,h); v = label;
    } else if (row[@"group"]) {
        UILabel *label = [UILabel new]; label.text = row[@"group"]; label.textColor = UIColor.labelColor;
        UIFont *base = [UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline];
        label.font = [UIFont systemFontOfSize:base.pointSize weight:UIFontWeightSemibold]; label.frame = CGRectMake(8,h-30,W-16,24); v = label;
    } else if (row[@"snippet"]) v = [self card:row frame:CGRectMake(0,3,W,h-6)];
    else if (row[@"empty"]) {
        UIView *box = [[UIView alloc] initWithFrame:CGRectMake(0,0,W,h)];
        UIImageView *glyph = [[UIImageView alloc] initWithImage:[UIImage systemImageNamed:row[@"symbol"] withConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:34 weight:UIImageSymbolWeightLight]]];
        glyph.tintColor = UIColor.quaternaryLabelColor; glyph.contentMode = UIViewContentModeCenter; glyph.frame = CGRectMake(0,h*0.42-56,W,48); [box addSubview:glyph];
        UILabel *label = [UILabel new]; label.text = row[@"empty"]; label.textColor = UIColor.secondaryLabelColor; label.numberOfLines = 0;
        label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline]; label.textAlignment = NSTextAlignmentCenter;
        label.frame = CGRectMake(24,h*0.42,W-48,44); [box addSubview:label]; v = box;
    } else if (row[@"media"]) {
        UIView *box = [[UIView alloc] initWithFrame:CGRectMake(0,0,W,h)]; CGFloat side = (W-10)/2;
        NSArray *items = row[@"media"];
        for (NSUInteger j = 0; j < items.count && j < 2; ++j) [box addSubview:[self tile:items[j] frame:CGRectMake(j*(side+10),0,side,h-10)]];
        v = box;
    } else {
        NSArray *items = row[@"items"];
        if ([[items firstObject][@"outline"] boolValue] && [[items firstObject][@"on"] boolValue]) {
            UIView *mark = [[UIView alloc] initWithFrame:CGRectMake(0,2,W,h-4)]; mark.tag = 101;
            mark.backgroundColor = UIColor.tertiarySystemFillColor; mark.layer.cornerRadius = 10; mark.layer.cornerCurve = kCACornerCurveContinuous;
            [cell.contentView addSubview:mark];
        }
        CGFloat indent = MIN([row[@"indent"] doubleValue],W/3);
        UIView *line = [self line:items width:W-indent height:h];
        line.frame = CGRectMake(indent,0,W-indent,h); v = line;
    }
    v.tag = 101; [cell.contentView addSubview:v];
    return cell;
}
// 下拉收起：从标题区往下拖，或列表已在顶端时往下拖，纸跟手下移；松手够远或够快就关闭，否则弹回。
- (BOOL)gestureRecognizerShouldBegin:(UIGestureRecognizer *)g {
    if (g != self.pull) return [super gestureRecognizerShouldBegin:g];
    CGPoint speed = [self.pull velocityInView:self];
    if (speed.y <= 0 || fabs(speed.x) > speed.y) return NO;
    UIView *list = self.list.superview ? self.list : nil;
    self.fromList = list && [list pointInside:[self.pull locationInView:list] withEvent:nil];
    return !self.fromList || self.list.contentOffset.y <= -self.list.adjustedContentInset.top+1;
}
- (BOOL)gestureRecognizer:(UIGestureRecognizer *)g shouldRecognizeSimultaneouslyWithGestureRecognizer:(UIGestureRecognizer *)other {
    return g == self.pull && other == self.list.panGestureRecognizer;
}
- (void)settle {
    [UIView animateWithSpringDuration:0.45 bounce:0.12 initialSpringVelocity:0 delay:0 options:UIViewAnimationOptionAllowUserInteraction|UIViewAnimationOptionBeginFromCurrentState animations:^{
        self.transform = CGAffineTransformIdentity; self.backdrop.alpha = 1;
    } completion:nil];
}
- (void)pulling:(UIPanGestureRecognizer *)g {
    CGFloat dy = [g translationInView:self.superview].y, h = MAX(1,self.bounds.size.height);
    if (g.state == UIGestureRecognizerStateBegan || g.state == UIGestureRecognizerStateChanged) {
        CGFloat t = dy > 0 ? dy : -pow(-dy,0.7);
        if (self.fromList) self.list.contentOffset = CGPointMake(0,-self.list.adjustedContentInset.top);
        self.transform = CGAffineTransformMakeTranslation(0,t); self.backdrop.alpha = 1-MAX(0,t)/h;
        return;
    }
    CGFloat speed = [g velocityInView:self.superview].y;
    if (g.state == UIGestureRecognizerStateEnded && (dy > 110 || (speed > 900 && dy > 16))) {
        self.pulled = YES; [self tappedBackdrop];
        __weak QuietEditorSurface *weak = self;
        // Rust 关掉面板后由 hide 接着滑下去；没关掉（比如正在拼音输入）就弹回。
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW,(int64_t)(0.35*NSEC_PER_SEC)),dispatch_get_main_queue(),^{
            if (weak.pulled && weak.userInteractionEnabled) { weak.pulled = NO; [weak settle]; }
        });
    } else [self settle];
}
@end

static NSMutableDictionary<NSString *, QuietEditorSurface *> *surfaces;
void quiet_editor_surface(UIView *parent, const char *key, const char *model, double x, double y, double width, double height, bool dark) {
    if (@available(iOS 26.0,*)) {
        if (!surfaces) surfaces = [NSMutableDictionary dictionary];
        NSString *name = @(key); QuietEditorSurface *v = surfaces[name]; BOOL rise = NO;
        NSString *sibling = [name isEqual:@"find"] ? @"catalogue" : ([name isEqual:@"catalogue"] ? @"find" : nil);
        if (!v && sibling && surfaces[sibling]) {
            // 目录和查找之间切换：接过同一张纸，旧 key 下一帧再 hide 时已经找不到它。
            v = surfaces[sibling]; [surfaces removeObjectForKey:sibling]; surfaces[name] = v;
            v.key = name; v.accessibilityIdentifier = [@"quiet.editor." stringByAppendingString:name]; v.adopted = YES;
        }
        if (!v) {
            BOOL page = [@[@"find",@"catalogue",@"language",@"equation"] containsObject:name], sheet = sheetKey(name);
            v = [[QuietEditorSurface alloc] initWithEffect:page ? nil : [UIGlassEffect effectWithStyle:UIGlassEffectStyleRegular]];
            v.sheet = sheet; v.inset = sheet ? 16 : 10; rise = sheet;
            if (page) { v.backgroundColor = UIColor.systemBackgroundColor; v.layer.cornerRadius = sheet ? 34 : 22; v.layer.cornerCurve = kCACornerCurveContinuous; v.layer.maskedCorners = kCALayerMinXMinYCorner | kCALayerMaxXMinYCorner; }
            if (page) {
                // 纸的底边停在键盘顶上；iOS 26 的键盘四角是圆的，往下垫一截同色的纸，圆角外露出的就是纸而不是后面压暗的页面（照 Craft）。
                // 不裁剪纸本身（内容在自己裁剪的滚动视图里），顶上的圆角由背景的 cornerRadius 画出。
                UIView *skirt = [UIView new]; skirt.tag = 'skrt';
                skirt.backgroundColor = UIColor.systemBackgroundColor; skirt.userInteractionEnabled = NO;
                [v.contentView addSubview:skirt];
            }
            if (page) {
                UIControl *backdrop = [[UIControl alloc] initWithFrame:parent.bounds];
                backdrop.autoresizingMask = UIViewAutoresizingFlexibleWidth|UIViewAutoresizingFlexibleHeight;
                backdrop.backgroundColor=[UIColor.blackColor colorWithAlphaComponent:sheet ? 0.2 : 0.26];
                [backdrop addGestureRecognizer:[[UITapGestureRecognizer alloc] initWithTarget:v action:@selector(tappedBackdrop)]];
                [parent addSubview:backdrop]; v.backdrop=backdrop;
            }
            v.key = name; v.accessibilityIdentifier = [@"quiet.editor." stringByAppendingString:name]; v.fields = [NSMutableDictionary dictionary]; v.lines = [NSMutableArray array];
            if (!page) v.cornerConfiguration = [UICornerConfiguration configurationWithUniformRadius:[UICornerRadius fixedRadius:22]];
            v.scroll = [UIScrollView new]; v.scroll.keyboardDismissMode = UIScrollViewKeyboardDismissModeNone;
            v.scroll.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
            if (sheet) { v.pull = [[UIPanGestureRecognizer alloc] initWithTarget:v action:@selector(pulling:)]; v.pull.delegate = v; [v addGestureRecognizer:v.pull]; }
            [v.contentView addSubview:v.scroll]; [parent addSubview:v]; surfaces[name] = v;
        }
        // 用 bounds/center 摆位：升起、下拉时 transform 不是单位矩阵，frame 读回来的是变换后的值。
        BOOL resized = !CGSizeEqualToSize(v.bounds.size,CGSizeMake(width,height));
        if (resized) { v.bounds = CGRectMake(0,0,width,height); v.scroll.frame = v.bounds; [v.contentView viewWithTag:'skrt'].frame = CGRectMake(0,height,width,1200); }
        CGPoint center = CGPointMake(x+width/2,y+height/2);
        if (!CGPointEqualToPoint(v.center,center)) v.center = center;
        UIUserInterfaceStyle style = dark ? UIUserInterfaceStyleDark : UIUserInterfaceStyleLight;
        if (v.overrideUserInterfaceStyle != style) v.overrideUserInterfaceStyle = style;
        NSString *data = @(model);
        if (resized && [name isEqual:@"equation"]) {
            UIBezierPath *path = [UIBezierPath bezierPathWithRect:v.bounds];
            [path appendPath:[UIBezierPath bezierPathWithRect:CGRectMake(0,62,width,118)]];
            CAShapeLayer *mask = [CAShapeLayer layer]; mask.path = path.CGPath; mask.fillRule = kCAFillRuleEvenOdd; v.layer.mask = mask;
        }
        if (resized || ![v.model isEqual:data]) {
            NSArray *rows = [NSJSONSerialization JSONObjectWithData:[data dataUsingEncoding:NSUTF8StringEncoding] options:0 error:NULL];
            if (rows) [v update:rows width:MAX(1,width-2*v.inset)]; v.model = data;
        }
        if (rise) {
            v.transform = CGAffineTransformMakeTranslation(0,height+40); v.backdrop.alpha = 0;
            [UIView animateWithSpringDuration:0.5 bounce:0.06 initialSpringVelocity:0 delay:0 options:UIViewAnimationOptionAllowUserInteraction animations:^{
                v.transform = CGAffineTransformIdentity; v.backdrop.alpha = 1;
            } completion:nil];
        }
    }
}
void quiet_hide_editor_surface(const char *key) {
    NSString *name = @(key); QuietEditorSurface *v = surfaces[name]; if (!v) return;
    [surfaces removeObjectForKey:name]; UIControl *backdrop = v.backdrop;
    if (!v.sheet || !v.window) { [backdrop removeFromSuperview]; [v removeFromSuperview]; return; }
    // 收起：先交出焦点，纸从当前位置滑到底下，压暗同时淡出，结束后再移除。
    v.userInteractionEnabled = NO; backdrop.userInteractionEnabled = NO; v.pulled = NO; [v endEditing:YES];
    [UIView animateWithSpringDuration:0.4 bounce:0 initialSpringVelocity:0 delay:0 options:UIViewAnimationOptionBeginFromCurrentState animations:^{
        v.transform = CGAffineTransformMakeTranslation(0,v.bounds.size.height+40); backdrop.alpha = 0;
    } completion:^(__unused BOOL done) { [backdrop removeFromSuperview]; [v removeFromSuperview]; }];
}
