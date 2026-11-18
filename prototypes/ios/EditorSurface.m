// 编辑器里的原生玻璃。每个浮层独立滚动；更新选项时保留正在输入的控件与拼音组合串。
#import <UIKit/UIKit.h>
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
@interface QuietEditorSurface : UIVisualEffectView <UITextFieldDelegate, UITextViewDelegate, UITableViewDataSource, UITableViewDelegate>
@property NSString *key;
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
                UIImageView *glyph=[[UIImageView alloc] initWithImage:[UIImage systemImageNamed:i==3 ? @"magnifyingglass" : @"arrow.right"]];
                glyph.tintColor=UIColor.secondaryLabelColor; glyph.contentMode=UIViewContentModeCenter; glyph.frame=CGRectMake(0,0,38,36);
                t.leftView=glyph; t.clearButtonMode=UITextFieldViewModeWhileEditing;
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
- (UIView *)line:(NSArray *)items width:(CGFloat)width {
    UIScrollView *strip = [UIScrollView new]; strip.showsHorizontalScrollIndicator = NO;
    CGFloat x = 0;
    for (NSDictionary *item in items) {
        UIButtonConfiguration *c = UIButtonConfiguration.plainButtonConfiguration;
        NSString *symbol = item[@"symbol"], *title = item[@"title"];
        BOOL iconOnly = [item[@"iconOnly"] boolValue], single = items.count == 1;
        c.title = iconOnly ? nil : title;
        if ([item[@"label"] boolValue]) {
            UILabel *label = [UILabel new]; label.text = title; label.textColor=UIColor.secondaryLabelColor; label.font=[UIFont preferredFontForTextStyle:UIFontTextStyleFootnote];
            CGFloat span=[label sizeThatFits:CGSizeMake(width,44)].width+8; label.frame=CGRectMake(x+4,0,span,44); [strip addSubview:label]; x+=span; continue;
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
        if ([item[@"on"] boolValue]) c.background.backgroundColor = UIColor.tertiarySystemFillColor;
        c.baseForegroundColor = [item[@"danger"] boolValue] ? UIColor.systemRedColor : ([item[@"on"] boolValue] ? self.tintColor : UIColor.labelColor);
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
        if ([item[@"on"] boolValue]) b.accessibilityTraits |= UIAccessibilityTraitSelected;
        if ([item[@"children"] count]) {
            NSMutableArray *actions = [NSMutableArray array];
            for (NSDictionary *child in item[@"children"]) {
                UIAction *a = [self action:child owner:b]; if ([child[@"disabled"] boolValue]) a.attributes=UIMenuElementAttributesDisabled; a.state = [child[@"on"] boolValue] ? UIMenuElementStateOn : UIMenuElementStateOff;
                [actions addObject:a];
            }
            b.menu = [UIMenu menuWithChildren:actions]; b.showsMenuAsPrimaryAction = YES;
        } else [b addAction:[self action:item owner:b] forControlEvents:UIControlEventTouchUpInside];
        CGFloat span = single ? width : MAX(44, iconOnly ? 44 : [b sizeThatFits:CGSizeMake(width,44)].width);
        if ([item[@"equal"] boolValue]) span = width/items.count;
        if ([item[@"trailing"] boolValue]) { if (single) span=44; x = MAX(x,width-span-[item[@"reserve"] doubleValue]); }
        if ([item[@"fill"] boolValue]) span = MAX(44,width-x-[item[@"reserve"] doubleValue]);
        b.titleLabel.lineBreakMode = NSLineBreakByTruncatingTail;
        if (single || [item[@"fill"] boolValue]) b.contentHorizontalAlignment = UIControlContentHorizontalAlignmentLeading;
        if ([self.key isEqual:@"selection"] && ( [symbol isEqual:@"plus.circle"] || [symbol isEqual:@"function"] )) b.contentHorizontalAlignment = UIControlContentHorizontalAlignmentCenter;
        b.frame = CGRectMake(x,0,span,[item[@"stack"] boolValue] ? 80 : 44); [strip addSubview:b];
        x += span;
    }
    strip.contentSize = CGSizeMake(x,44); strip.scrollEnabled = x > width; return strip;
}
- (void)update:(NSArray *)rows width:(CGFloat)w {
    NSArray *old = self.rows; self.rows = rows;
    CGFloat y = 6;
    NSUInteger boundary = [rows indexOfObjectPassingTest:^BOOL(NSDictionary *r, __unused NSUInteger idx, __unused BOOL *stop) { return [r[@"list"] boolValue]; }];
    BOOL listed = boundary != NSNotFound;
    self.listStart = listed ? boundary+1 : rows.count;
    NSMutableSet *keep = [NSMutableSet set];
    for (NSInteger i = 0; i < self.listStart; ++i) {
        NSDictionary *r = rows[i];
        CGFloat h = r[@"h"] ? [r[@"h"] doubleValue] : (r[@"caption"] ? 28 : ([r[@"multi"] boolValue] ? 96 : 44));
        if ([r[@"stretch"] boolValue] || [r[@"multi"] boolValue]) h = MAX(44,self.bounds.size.height-y-54);
        UIView *v;
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
        else if (i < (NSInteger)old.count && [old[i] isEqual:r] && self.lines[i].frame.size.width == w) v = self.lines[i];
        else if (r[@"header"]) {
            UIView *header = [UIView new]; BOOL source = [r[@"source"] boolValue];
            UILabel *label = [UILabel new]; label.text = r[@"header"]; label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleHeadline];
            label.frame = CGRectMake(source ? 48 : 4,0,w-(source ? 96 : 52),h); label.textAlignment = source ? NSTextAlignmentCenter : NSTextAlignmentLeft;
            if (r[@"space"]) {
                UIImageView *doc = [[UIImageView alloc] initWithImage:[UIImage systemImageNamed:@"doc.text.fill"]];
                doc.contentMode=UIViewContentModeScaleAspectFit; doc.tintColor=UIColor.secondaryLabelColor;
                doc.frame=CGRectMake(4,14,32,44); [header addSubview:doc];
                label.frame=CGRectMake(44,14,w-96,24);
                UILabel *space=[UILabel new]; space.text=r[@"space"]; space.font=[UIFont preferredFontForTextStyle:UIFontTextStyleCaption1];
                space.textColor=UIColor.secondaryLabelColor; space.frame=CGRectMake(44,38,w-96,18); [header addSubview:space];
            }
            [header addSubview:label]; NSDictionary *item = [r[@"items"] firstObject];
            UIButton *close = [UIButton buttonWithType:UIButtonTypeSystem]; [close setImage:[UIImage systemImageNamed:source ? @"checkmark.circle.fill" : @"xmark.circle.fill"] forState:UIControlStateNormal];
            close.tintColor = source ? self.tintColor : UIColor.tertiaryLabelColor;
            [close setPreferredSymbolConfiguration:[UIImageSymbolConfiguration configurationWithPointSize:source ? 33 : 23] forImageInState:UIControlStateNormal];
            close.accessibilityIdentifier = [@"quiet.close." stringByAppendingString:self.key];
            close.frame = CGRectMake(w-44,(h-44)/2,44,44); close.accessibilityLabel = item[@"title"];
            [header addSubview:close]; [close addAction:[self action:item owner:close] forControlEvents:UIControlEventTouchUpInside]; v = header;
        } else if ([r[@"style"] isEqual:@"tabs"]) {
            NSArray *items = r[@"items"]; UISegmentedControl *tabs = [UISegmentedControl new];
            for (NSUInteger j=0;j<items.count;j++) {
                NSDictionary *item = items[j]; [tabs insertSegmentWithAction:[self action:item owner:tabs] atIndex:j animated:NO];
                if ([item[@"on"] boolValue]) tabs.selectedSegmentIndex = j;
            }
            v = tabs;
        } else if (r[@"caption"]) {
            UILabel *label = [UILabel new]; label.text = r[@"caption"]; label.textColor = UIColor.secondaryLabelColor;
            label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleFootnote]; v = label;
        } else v = [self line:r[@"items"] width:w];
        if (i < (NSInteger)self.lines.count) {
            UIView *prior = self.lines[i];
            if (prior != v && ![self.fields.allValues containsObject:prior]) {
                if ([prior isKindOfClass:UIScrollView.class] && [v isKindOfClass:UIScrollView.class])
                    ((UIScrollView *)v).contentOffset = ((UIScrollView *)prior).contentOffset;
                [prior removeFromSuperview];
            }
            self.lines[i] = v;
        } else [self.lines addObject:v];
        if (!v.superview) [self.scroll addSubview:v];
        if ([r[@"multi"] boolValue]) {
            v.layer.cornerRadius=0; v.frame=CGRectMake(0,y,self.bounds.size.width,h);
        } else if (r[@"field"] && [self.key isEqual:@"find"]) v.frame=CGRectMake(10,y+3,w,h-6);
        else v.frame = CGRectMake(10,y,w,h);
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
    if (listed) {
        if (!self.list) {
            self.list = [[UITableView alloc] initWithFrame:CGRectZero style:UITableViewStylePlain];
            self.list.dataSource = self; self.list.delegate = self;
            self.list.separatorStyle = UITableViewCellSeparatorStyleNone; self.list.backgroundColor = UIColor.clearColor;
            self.list.allowsSelection = NO; self.list.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
            [self.scroll addSubview:self.list]; self.activeRow = NSNotFound;
        }
        self.list.frame = CGRectMake(10,y,w,MAX(0,self.bounds.size.height-y-6));
        self.list.contentInset=UIEdgeInsetsMake(0,0,self.safeAreaInsets.bottom,0); [self.list reloadData];
        NSInteger active = NSNotFound;
        for (NSInteger i = self.listStart; i < (NSInteger)rows.count; ++i) {
            if ([[rows[i][@"items"] lastObject][@"on"] boolValue]) { active = i-self.listStart; break; }
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
- (CGFloat)tableView:(UITableView *)table heightForRowAtIndexPath:(NSIndexPath *)path { return 44; }
- (UITableViewCell *)tableView:(UITableView *)table cellForRowAtIndexPath:(NSIndexPath *)path {
    UITableViewCell *cell = [table dequeueReusableCellWithIdentifier:@"option"];
    if (!cell) cell = [[UITableViewCell alloc] initWithStyle:UITableViewCellStyleDefault reuseIdentifier:@"option"];
    cell.backgroundColor = UIColor.clearColor;
    [[cell.contentView viewWithTag:101] removeFromSuperview];
    NSDictionary *row = self.rows[self.listStart + path.row];
    if (row[@"caption"]) {
        cell.textLabel.text = row[@"caption"]; cell.textLabel.textColor = UIColor.secondaryLabelColor;
    } else {
        cell.textLabel.text = nil;
        CGFloat indent = MIN([row[@"indent"] doubleValue],table.bounds.size.width/3);
        UIView *line = [self line:row[@"items"] width:table.bounds.size.width-indent];
        line.tag = 101; line.frame = CGRectMake(indent,0,table.bounds.size.width-indent,44);
        [cell.contentView addSubview:line];
    }
    return cell;
}
@end

static NSMutableDictionary<NSString *, QuietEditorSurface *> *surfaces;
void quiet_editor_surface(UIView *parent, const char *key, const char *model, double x, double y, double width, double height, bool dark) {
    if (@available(iOS 26.0,*)) {
        if (!surfaces) surfaces = [NSMutableDictionary dictionary];
        NSString *name = @(key); QuietEditorSurface *v = surfaces[name];
        if (!v) {
            BOOL page = [@[@"find",@"catalogue",@"language",@"equation"] containsObject:name];
            v = [[QuietEditorSurface alloc] initWithEffect:page ? nil : [UIGlassEffect effectWithStyle:UIGlassEffectStyleRegular]];
            if (page) { v.backgroundColor = UIColor.systemBackgroundColor; v.layer.cornerRadius = 22; v.layer.maskedCorners = kCALayerMinXMinYCorner | kCALayerMaxXMinYCorner; v.clipsToBounds = YES; }
            if (page) {
                UIControl *backdrop = [[UIControl alloc] initWithFrame:parent.bounds];
                backdrop.autoresizingMask = UIViewAutoresizingFlexibleWidth|UIViewAutoresizingFlexibleHeight;
                backdrop.backgroundColor=[UIColor.blackColor colorWithAlphaComponent:0.26];
                [backdrop addGestureRecognizer:[[UITapGestureRecognizer alloc] initWithTarget:v action:@selector(tappedBackdrop)]];
                [parent addSubview:backdrop]; v.backdrop=backdrop;
            }
            v.key = name; v.accessibilityIdentifier = [@"quiet.editor." stringByAppendingString:name]; v.fields = [NSMutableDictionary dictionary]; v.lines = [NSMutableArray array];
            if (!page) v.cornerConfiguration = [UICornerConfiguration configurationWithUniformRadius:[UICornerRadius fixedRadius:22]];
            v.scroll = [UIScrollView new]; v.scroll.keyboardDismissMode = UIScrollViewKeyboardDismissModeNone;
            v.scroll.contentInsetAdjustmentBehavior = UIScrollViewContentInsetAdjustmentNever;
            [v.contentView addSubview:v.scroll]; [parent addSubview:v]; surfaces[name] = v;
        }
        BOOL resized = !CGSizeEqualToSize(v.bounds.size,CGSizeMake(width,height));
        CGRect frame = CGRectMake(x,y,width,height);
        if (!CGRectEqualToRect(v.frame,frame)) { v.frame = frame; v.scroll.frame = v.bounds; }
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
            if (rows) [v update:rows width:MAX(1,width-20)]; v.model = data;
        }
    }
}
void quiet_hide_editor_surface(const char *key) {
    NSString *name = @(key); [surfaces[name].backdrop removeFromSuperview]; [surfaces[name] removeFromSuperview]; [surfaces removeObjectForKey:name];
}
